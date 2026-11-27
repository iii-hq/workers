//! `check` on real temp repositories: the plan statuses, and what "ready" pins.

mod support;

use iii_cloud::check::check;
use iii_cloud::project::use_project;
use iii_cloud::types::*;
use serde_json::json;
use support::Harness;
use support::repo::*;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn ready(server: &MockServer, plan_status: u16) -> (Harness, Repo, std::path::PathBuf) {
    ready_with(server, plan_status, std::time::Duration::ZERO).await
}

/// `ready`, with the cloud answering the plan after `plan_delay`.
async fn ready_with(
    server: &MockServer,
    plan_status: u16,
    plan_delay: std::time::Duration,
) -> (Harness, Repo, std::path::PathBuf) {
    let repo = repo();
    let h = connected(server, "worker-compose.yaml").await;
    Mock::given(method("GET"))
        .and(path("/api/v1/support-matrix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"engine": "0.27.0", "minimum_sdk": "0.26"})),
        )
        .mount(server)
        .await;
    let body = if plan_status == 200 {
        json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": []}})
    } else {
        json!({"code": "NOT_FOUND", "message": "No such commit in this repository."})
    };
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(plan_status)
                .set_body_json(body)
                .set_delay(plan_delay),
        )
        .mount(server)
        .await;
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
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    (h, repo, ws)
}

fn codes(c: &Check) -> Vec<&str> {
    c.problems.iter().map(|p| p.code.as_str()).collect()
}

#[tokio::test]
async fn a_pushed_clean_commit_has_a_clean_plan_and_is_not_ready_without_a_run() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::Clean);
    assert_eq!(c.commit, git(&ws, &["rev-parse", "HEAD"]));
    assert_eq!(c.base, git(&repo.origin, &["rev-parse", "main"]));
    assert!(
        !codes(&c).contains(&"UNCOMMITTED_CHANGES") && !codes(&c).contains(&"BASE_NOT_IN_HISTORY")
    );
    assert!(!c.ready, "no run yet");
    assert!(
        c.report
            .starts_with("## iii cloud readiness\nCloud plan for ")
    );
    assert_eq!(h.ctx.store.read(|s| s.check.clone()).unwrap().id, c.id);
    assert!(h.fake.kinds().contains(&"check:true".to_string()));
}

#[tokio::test]
async fn uncommitted_changes_and_a_base_that_moved_block_ready() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    std::fs::write(ws.join("dirty.txt"), "x").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(
        codes(&c).contains(&"UNCOMMITTED_CHANGES"),
        "{:?}",
        codes(&c)
    );
    // The run's own files do not count as changes.
    std::fs::remove_file(ws.join("dirty.txt")).unwrap();
    std::fs::write(ws.join("worker-compose.iii-cloud-run.yaml"), "x").unwrap();
    std::fs::write(ws.join(".env"), "A=1").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(
        !codes(&c).contains(&"UNCOMMITTED_CHANGES"),
        "{:?}",
        codes(&c)
    );
    // main moves on: its head is not in the branch's history.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "c"],
    );
    let c2 = t.path().join("c");
    std::fs::write(c2.join("x"), "x").unwrap();
    git(&c2, &["add", "."]);
    git(&c2, &["commit", "-m", "moved"]);
    git(&c2, &["push", "-q", "origin", "main"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let problem = c
        .problems
        .iter()
        .find(|p| p.code == "BASE_NOT_IN_HISTORY")
        .expect("a moved base is a problem");
    assert_eq!(
        (problem.owner, problem.severity, problem.source),
        (Owner::Agent, Severity::Block, Source::CheckHere)
    );
}

#[tokio::test]
async fn a_commit_github_does_not_have_is_not_pushed_and_asks_the_cloud_nothing() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    std::fs::write(ws.join("f.txt"), "f").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "local"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::NotPushed);
    assert!(c.report.contains("not pushed to GitHub"));
    assert!(
        c.here.problems as usize <= c.problems.len(),
        "the checks here still ran"
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().ends_with("/plan"))
    );
}

#[tokio::test]
async fn a_404_on_a_pushed_commit_is_commit_not_planned() {
    let server = MockServer::start().await;
    let (h, _repo, _ws) = ready(&server, 404).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::CommitNotPlanned);
}

#[tokio::test]
async fn check_needs_a_selected_environment_and_a_credential() {
    let server = MockServer::start().await;
    let h = connected(&server, "worker-compose.yaml").await;
    assert_eq!(
        check(&h.ctx, CheckRequest::default())
            .await
            .unwrap_err()
            .code,
        "NO_PROJECT"
    );
    let h2 = support::harness(&server.uri(), std::sync::Arc::new(support::Fake::default()));
    assert_eq!(
        check(&h2.ctx, CheckRequest::default())
            .await
            .unwrap_err()
            .code,
        "NO_PROJECT"
    );
}

#[tokio::test]
async fn an_unreadable_base_is_a_problem_not_a_pass() {
    let server = MockServer::start().await;
    let (h, repo, _ws) = ready(&server, 200).await;
    git(
        &repo.checkout,
        &[
            "config",
            "--unset",
            &format!("url.{}.insteadOf", repo.origin.display()),
        ],
    );
    git(
        &repo.checkout,
        &["remote", "set-url", "origin", "/nonexistent/origin.git"],
    );
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "BASE_NOT_IN_HISTORY")
        .expect("the gate does not fail open");
    assert!(p.title.contains("Could not read"));
    assert!(!c.ready && c.base.is_empty());
}

#[tokio::test]
async fn a_short_commit_is_stored_in_full_so_ready_can_equal_the_runs_head() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let full = git(&ws, &["rev-parse", "HEAD"]);
    let c = check(
        &h.ctx,
        CheckRequest {
            handoff_id: None,
            commit: Some(full[..9].to_string()),
            run: None,
            manifest: None,
            push: None,
            force: None,
            from_page: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(c.commit, full);
    // A commit only the cloud knows stays as given.
    let c = check(
        &h.ctx,
        CheckRequest {
            handoff_id: None,
            commit: Some("abcdef1".into()),
            run: None,
            manifest: None,
            push: None,
            force: None,
            from_page: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(c.commit, "abcdef1");
    assert!(c.commits.is_empty(), "not here: no commit list");
}

fn plan_mock(status: u16, body: serde_json::Value) -> Mock {
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .with_priority(1)
}

#[tokio::test]
async fn a_404_that_is_not_about_the_commit_is_unavailable_with_the_clouds_message() {
    let server = MockServer::start().await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    plan_mock(
        404,
        json!({"code": "NOT_FOUND", "message": "No such environment."}),
    )
    .mount(&server)
    .await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::Unavailable);
    assert!(
        c.plan
            .error
            .unwrap()
            .message
            .contains("No such environment")
    );
}

#[tokio::test]
async fn the_commit_goes_to_the_cloud_in_lowercase() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let full = git(&ws, &["rev-parse", "HEAD"]);
    // An uppercase sha, as a person might paste it.
    let c = check(
        &h.ctx,
        CheckRequest {
            handoff_id: None,
            commit: Some(full[..10].to_uppercase()),
            run: None,
            manifest: None,
            push: None,
            force: None,
            from_page: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(c.commit, full, "stored in full, lowercase");
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/plan"))
        .filter_map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == "commit")
                .map(|(_, v)| v.into_owned())
        })
        .collect();
    assert!(
        !asked.is_empty() && asked.iter().all(|a| *a == a.to_lowercase()),
        "{asked:?}"
    );
    // A commit only the cloud knows, uppercase: lowercase as well.
    check(
        &h.ctx,
        CheckRequest {
            handoff_id: None,
            commit: Some("ABCDEF1".into()),
            run: None,
            manifest: None,
            push: None,
            force: None,
            from_page: false,
        },
    )
    .await
    .unwrap();
    let last = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .rev()
        .find(|r| r.url.path().ends_with("/plan"))
        .unwrap()
        .url
        .query()
        .unwrap()
        .to_string();
    assert!(last.contains("commit=abcdef1"), "{last}");
}

#[tokio::test]
async fn a_plan_of_another_commit_is_unavailable() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let full = git(&ws, &["rev-parse", "HEAD"]);
    let plan = |commit: &str| json!({"can_deploy": true, "fingerprint": "fp", "plan": {"source": {"commit": commit}, "settings": {}, "resources": [], "issues": [], "deploy_issues": []}});
    // The cloud answered with the plan of another commit.
    plan_mock(200, plan("ffffffffffffffffffffffffffffffffffffffff"))
        .mount(&server)
        .await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::Unavailable);
    assert!(c.plan.error.unwrap().message.contains("instead of"));
    // Its own commit (the cloud may answer a longer sha): fine.
    server.reset().await;
    let h2 = {
        // A fresh server for the matching plan.
        let s2 = MockServer::start().await;
        let (h2, _r, ws2) = ready(&s2, 200).await;
        let full2 = git(&ws2, &["rev-parse", "HEAD"]);
        plan_mock(200, plan(&full2)).mount(&s2).await;
        let c = check(&h2.ctx, CheckRequest::default()).await.unwrap();
        assert_eq!(c.plan.status, PlanStatus::Clean);
        h2
    };
    let _ = (h2, full);
}

/// The plan requests that named `manifest`, in order.
async fn asked_manifests(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .unwrap()
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

/// A second compose file, committed; its head is checked by sha so the plan is asked.
fn commit_next_file(ws: &std::path::Path) -> String {
    std::fs::write(
        ws.join("worker-compose-next.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      TOKEN: ${ONLY_IN_NEXT}\ncloud: {}\n",
    )
    .unwrap();
    git(ws, &["add", "-A"]);
    git(ws, &["commit", "-m", "next"]);
    git(ws, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn another_compose_file_is_planned_and_checked_here_and_kept_on_the_check() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let head = commit_next_file(&ws);
    // The cloud plans the file it was asked for.
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(|r: &wiremock::Request| {
            let m = r
                .url
                .query_pairs()
                .find(|(k, _)| k == "manifest")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_else(|| "worker-compose.yaml".into());
            ResponseTemplate::new(200).set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"source": {"manifest": m}, "settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))
        })
        .with_priority(1)
        .mount(&server)
        .await;
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head.clone()),
            manifest: Some("worker-compose-next.yaml".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.plan.status, PlanStatus::Clean);
    assert_eq!(c.manifest.as_deref(), Some("worker-compose-next.yaml"));
    assert_eq!(
        asked_manifests(&server).await,
        vec![Some("worker-compose-next.yaml".to_string())]
    );
    // The checks here read that file too: only it references ONLY_IN_NEXT.
    assert!(
        c.problems.iter().any(|p| p.title.contains("ONLY_IN_NEXT")),
        "{:?}",
        codes(&c)
    );
    // A check that names no file stays on the newest check's.
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.manifest.as_deref(), Some("worker-compose-next.yaml"));
    assert_eq!(
        asked_manifests(&server).await.last(),
        Some(&Some("worker-compose-next.yaml".to_string()))
    );
    // The environment's own file, named, is the way back: no ask to the cloud.
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head.clone()),
            manifest: Some("worker-compose.yaml".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.manifest.as_deref(), Some("worker-compose.yaml"));
    assert_eq!(asked_manifests(&server).await.last(), Some(&None));
    assert!(!c.problems.iter().any(|p| p.title.contains("ONLY_IN_NEXT")));
    // And a plain check after it stays there.
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.manifest.as_deref(), Some("worker-compose.yaml"));
    assert_eq!(asked_manifests(&server).await.last(), Some(&None));
}

#[tokio::test]
async fn a_cloud_that_plans_the_environments_file_instead_is_a_cloud_error() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let head = commit_next_file(&ws);
    plan_mock(200, json!({"can_deploy": true, "fingerprint": "fp", "plan": {"source": {"manifest": "worker-compose.yaml"}, "settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))
        .mount(&server)
        .await;
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head),
            manifest: Some("worker-compose-next.yaml".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.plan.status, PlanStatus::Unavailable);
    let e = c.plan.error.clone().unwrap();
    assert_eq!(e.code, "CLOUD_ERROR");
    assert_eq!(
        e.message,
        "iii cloud planned worker-compose.yaml instead of worker-compose-next.yaml."
    );
    assert!(!c.ready);
    // Without a plan the lock is looked for here, beside the asked file.
    assert!(
        c.problems
            .iter()
            .any(|p| p.code == "LOCK_MISSING"
                && p.file.as_deref() == Some("worker-compose-next.lock")),
        "{:?}",
        c.problems
    );
}

#[tokio::test]
async fn a_manifest_the_cloud_would_not_plan_is_refused_before_anything_runs() {
    for ok in [
        "worker-compose.yaml",
        "api/worker-compose-prod.yaml",
        "worker-compose.next.yaml",
    ] {
        assert!(iii_cloud::local::manifest_path(ok), "{ok}");
    }
    let server = MockServer::start().await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    for bad in [
        "docker-compose.yaml",
        "workers-compose.yaml",
        "worker-compose.yml",
        "../worker-compose.yaml",
        "/worker-compose.yaml",
        "api//worker-compose.yaml",
        "api/./worker-compose.yaml",
        "api/worker-compose.yaml/",
        "",
    ] {
        let err = check(
            &h.ctx,
            CheckRequest {
                manifest: Some(bad.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST", "{bad:?}");
    }
    assert!(
        asked_manifests(&server).await.is_empty(),
        "no plan was asked"
    );
    assert!(h.ctx.store.read(|s| s.check.is_none()), "no check was kept");
}

#[tokio::test]
async fn a_plan_that_does_not_say_its_file_is_not_taken_for_the_asked_one() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let head = commit_next_file(&ws);
    plan_mock(200, json!({"can_deploy": true, "fingerprint": "fp", "plan": {"source": {"commit": head}, "settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))
        .mount(&server)
        .await;
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head),
            manifest: Some("worker-compose-next.yaml".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.plan.status, PlanStatus::Unavailable);
    let e = c.plan.error.unwrap();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        (
            "CLOUD_ERROR",
            "iii cloud planned the environment's file instead of worker-compose-next.yaml."
        )
    );
}

#[tokio::test]
async fn a_cloud_environment_with_no_file_is_checked_with_none_not_the_stored_one() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    // The stored record still names a file the cloud no longer has set.
    std::fs::create_dir_all(ws.join("api")).unwrap();
    std::fs::write(
        ws.join("api/worker-compose-prod.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      TOKEN: ${ONLY_IN_PROD}\n",
    )
    .unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "prod"]);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = Some("api/worker-compose-prod.yaml".into());
    });
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": null, "url": null, "settings": {}})))
        .with_priority(1)
        .mount(&server)
        .await;
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(head),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // The cloud planned its own setting (none): the check names none either,
    // and the checks here did not read the stored file.
    assert_eq!(c.manifest, None);
    assert_eq!(asked_manifests(&server).await, vec![None]);
    assert!(!c.problems.iter().any(|p| p.title.contains("ONLY_IN_PROD")));
    // Asking for the stored file is then a real ask.
    let c = check(
        &h.ctx,
        CheckRequest {
            commit: Some(git(&ws, &["rev-parse", "HEAD"])),
            manifest: Some("api/worker-compose-prod.yaml".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        asked_manifests(&server).await.last(),
        Some(&Some("api/worker-compose-prod.yaml".to_string()))
    );
    assert!(c.problems.iter().any(|p| p.title.contains("ONLY_IN_PROD")));
}

/// A hand-off with no worktree of its own: its checks use the global selection.
fn a_handoff(h: &Harness, id: &str) {
    let handoff: Handoff = serde_json::from_value(
        json!({"id": id, "marker": "m", "environment_id": ENV, "kind": "ready", "asked_at_ms": 0,
        "agent": {"handoff_id": id, "state": "working", "started_at": "t", "steps": 0}}),
    )
    .unwrap();
    h.ctx.store.update(|s| s.handoffs.push(handoff));
}

#[tokio::test]
async fn a_check_keeps_its_hand_off_the_commits_on_top_of_the_base_and_the_plans_summary() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    plan_mock(200, json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": [],
        "workers": [{"name": "api", "kind": "path"}, {"name": "http", "kind": "binary"}, {"name": "cron", "kind": "binary"}],
        "groups": [{"name": "app", "workers": ["api", "cron"]}, {"name": "web", "workers": ["http"]}]}}))
    .mount(&server)
    .await;
    for (file, message) in [("a.txt", "First fix"), ("b.txt", "Second fix")] {
        std::fs::write(ws.join(file), file).unwrap();
        git(&ws, &["add", "."]);
        git(&ws, &["commit", "-m", message]);
    }
    git(&ws, &["push", "-q", "origin", "HEAD"]);
    a_handoff(&h, "h1");
    let c = check(
        &h.ctx,
        CheckRequest {
            handoff_id: Some("h1".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.plan.status, PlanStatus::Clean);
    assert_eq!(c.handoff_id.as_deref(), Some("h1"));
    let (head, before) = (
        git(&ws, &["rev-parse", "HEAD"]),
        git(&ws, &["rev-parse", "HEAD~1"]),
    );
    assert_eq!(
        c.commits,
        vec![
            CommitLine {
                sha: head,
                message: "Second fix".into()
            },
            CommitLine {
                sha: before,
                message: "First fix".into()
            },
        ],
        "newest first, the base itself left out"
    );
    assert_eq!(
        c.plan.summary,
        Some(PlanSummary {
            workers: 3,
            groups: vec!["app".into(), "web".into()]
        })
    );
    // The page's own check names no hand-off; the base has nothing on top.
    let page = check(
        &h.ctx,
        CheckRequest {
            commit: Some(c.base.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.handoff_id, None);
    assert!(page.commits.is_empty());
    // A plan that never came has no summary.
    std::fs::write(ws.join("c.txt"), "c").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "Not pushed"]);
    let local = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(
        (local.plan.status, local.plan.summary),
        (PlanStatus::NotPushed, None)
    );
    assert_eq!(local.commits.len(), 3);
}

#[tokio::test]
async fn status_lists_every_kept_check_in_short_and_check_get_reads_one_whole() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    let first = check(&h.ctx, CheckRequest::default()).await.unwrap();
    std::fs::write(ws.join("dirty.txt"), "x").unwrap();
    let second = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let ids: Vec<&str> = status.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![second.id.as_str(), first.id.as_str()],
        "newest first"
    );
    assert_eq!(
        status.checks[0],
        CheckSummary {
            id: second.id.clone(),
            environment_id: ENV.into(),
            commit: second.commit.clone(),
            checked_at: second.checked_at.clone(),
            duration_ms: second.duration_ms,
            ready: false,
            problems: second
                .problems
                .iter()
                .filter(|p| p.owner != Owner::None)
                .count() as u32,
            handoff_id: None,
            manifest: second.manifest.clone(),
        }
    );
    assert_eq!(
        status.checks[0].problems,
        status.checks[1].problems + 1,
        "UNCOMMITTED_CHANGES is one more"
    );
    assert_eq!(
        iii_cloud::check::get(
            &h.ctx,
            CheckGetRequest {
                id: first.id.clone()
            }
        )
        .unwrap(),
        first
    );
    assert_eq!(
        iii_cloud::check::get(&h.ctx, CheckGetRequest { id: "nope".into() })
            .unwrap_err()
            .code,
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn push_sends_the_fix_branch_first_and_never_a_branch_an_environment_deploys() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    std::fs::write(ws.join("fix.txt"), "f").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "fix"]);
    let pushing = || CheckRequest {
        push: Some(true),
        ..Default::default()
    };
    // Without `push` the commit is not on GitHub; with it, the cloud plans it.
    let c = check(&h.ctx, pushing()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::Clean);
    assert_eq!(
        git(&repo.origin, &["rev-parse", "iii-cloud/ready-production"]),
        c.commit
    );
    // The commit the check resolved is pushed, not whatever HEAD is meanwhile.
    std::fs::write(ws.join("later.txt"), "l").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "later"]);
    let at = check(
        &h.ctx,
        CheckRequest {
            commit: Some(c.commit.clone()),
            push: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        git(&repo.origin, &["rev-parse", "iii-cloud/ready-production"]),
        at.commit,
        "not the later HEAD"
    );
    // An environment deploys that branch: refused, nothing pushed.
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .environments
            .push(EnvironmentBrief {
                id: "e2".into(),
                name: "preview".into(),
                production: false,
                branch: "iii-cloud/ready-production".into(),
            })
    });
    std::fs::write(ws.join("more.txt"), "m").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "more"]);
    let err = check(&h.ctx, pushing()).await.unwrap_err();
    assert_eq!(err.code, "PUSH_REFUSED");
    assert_eq!(
        git(&repo.origin, &["rev-parse", "iii-cloud/ready-production"]),
        c.commit,
        "origin is unchanged"
    );
    // Nor a branch that is not a fix branch (the person's own, say).
    git(&ws, &["checkout", "-q", "-b", "feature"]);
    assert_eq!(
        check(&h.ctx, pushing()).await.unwrap_err().code,
        "PUSH_REFUSED"
    );
}

/// `LOCAL_VARIABLE_UNSET` is what is left after the run here's fill: a name
/// the person's folder has in an env file is not raised, one with no source
/// anywhere is, and the value is nowhere in the answer, the state or an event.
#[tokio::test]
async fn only_a_name_with_no_source_here_is_local_variable_unset() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      GH_TOKEN: ${GH_TOKEN}\n      NO_SOURCE: ${NO_SOURCE}\n",
    )
    .unwrap();
    let value = "check-secret-value-0008";
    // The environment is production: its own file is `.env.production`.
    std::fs::write(
        repo.checkout.join(".env.production"),
        format!("GH_TOKEN={value}\n"),
    )
    .unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let unset: Vec<&str> = c
        .problems
        .iter()
        .filter(|p| p.code == "LOCAL_VARIABLE_UNSET")
        .map(|p| p.title.as_str())
        .collect();
    assert_eq!(unset, vec!["NO_SOURCE has no value here"]);
    let state = h.ctx.store.read(|s| serde_json::to_string(s).unwrap());
    let events = serde_json::to_string(&*h.fake.events.lock().unwrap()).unwrap();
    for text in [&serde_json::to_string(&c).unwrap(), &state, &events] {
        assert!(!text.contains(value));
    }
}

/// "Skip for now": the run here gives the name `""`, so it is no
/// `LOCAL_VARIABLE_UNSET` and the check can be ready; undo brings it back.
#[tokio::test]
async fn a_skipped_name_is_no_local_problem_and_the_check_can_be_ready_until_undone() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    // The test's own `iii`, not the machine's: a runner without iii 0.27 has `LOCAL_ENGINE_VERSION`,
    // which is no part of what this test is about and keeps the check from being ready.
    with_fast_runs(&h).await;
    // The compose file reads a name nothing gives, pushed to main.
    std::fs::write(
        repo.checkout.join("worker-compose.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      GH_TOKEN: ${GH_TOKEN}\ncloud:\n  healthcheck:\n    path: /health\n",
    )
    .unwrap();
    git(&repo.checkout, &["commit", "-qam", "reads GH_TOKEN"]);
    git(&repo.checkout, &["push", "-q", "origin", "main"]);
    git(&ws, &["fetch", "-q", "origin"]);
    git(&ws, &["merge", "-q", "--ff-only", "origin/main"]);
    let unset = |c: &Check| codes(c).contains(&"LOCAL_VARIABLE_UNSET");
    let passed = |c: &Check| {
        iii_cloud::check::is_ready(
            &c.plan,
            &c.problems,
            Some(&Run {
                id: "r".into(),
                check_id: c.id.clone(),
                commit: c.commit.clone(),
                engine: "0.27.0".into(),
                status: RunStatus::Passed,
                started_at: "t".into(),
                finished_at: None,
                stages: vec![],
                workers: WorkerCount { ready: 1, total: 1 },
                routes: RouteCount {
                    count: 0,
                    prefix: None,
                },
                healthcheck: None,
                not_covered: iii_cloud::check::default_not_covered(),
                log_tail: vec![],
                error: None,
                reused: false,
                waiting_on: None,
                restored: vec![],
                moved: vec![],
                log_path: None,
            }),
            &c.commit,
        )
    };
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(unset(&c) && !passed(&c));
    let skip = |names: &[&str], skip: bool| ValuesSkip {
        environment_id: ENV.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
        skip,
    };
    let out = iii_cloud::values::skip(&h.ctx, skip(&["GH_TOKEN"], true))
        .await
        .unwrap();
    assert_eq!(out.names, vec!["GH_TOKEN"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!unset(&c), "{:?}", codes(&c));
    assert!(passed(&c), "{:?}", codes(&c));
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert_eq!(status.skipped_values, vec!["GH_TOKEN"]);
    // Undo.
    let out = iii_cloud::values::skip(&h.ctx, skip(&["GH_TOKEN"], false))
        .await
        .unwrap();
    assert!(out.names.is_empty());
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(unset(&c));
    assert!(h.ctx.store.read(|s| s.skipped_values.is_empty()));
    // The guards: another environment, a name iii cloud refuses.
    let other = ValuesSkip {
        environment_id: "other".into(),
        ..skip(&["GH_TOKEN"], true)
    };
    let err = iii_cloud::values::skip(&h.ctx, other).await.unwrap_err();
    assert_eq!(err.code, "ENVIRONMENT_CHANGED");
    let err = iii_cloud::values::skip(&h.ctx, skip(&["lower"], true))
        .await
        .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
}

/// "Skip for now" of a cloud warning (`problem::skip`): the next check marks
/// `LOCAL_CONFIG_NOT_SET` skipped, as info; undo brings the warning back; a
/// code that isn't a skippable warning is refused.
#[tokio::test]
async fn a_skipped_cloud_warning_turns_to_info_until_undone_and_other_codes_are_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [
            {"code": "LOCAL_CONFIG_NOT_SET", "severity": "warn", "kind": "fix_in_repo", "message": "The workers' config is read from ./config, which isn't in the repository."}
        ], "deploy_issues": []}})))
        .with_priority(1)
        .mount(&server)
        .await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    let config = |c: &Check| {
        c.problems
            .iter()
            .find(|p| p.code == "LOCAL_CONFIG_NOT_SET")
            .map(|p| (p.severity, p.skipped))
    };
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(config(&c), Some((Severity::Warn, false)));
    assert_eq!(c.plan.warnings, 1);
    let skip = |code: &str, skip: bool| ProblemSkip {
        environment_id: ENV.into(),
        code: code.into(),
        skip,
    };
    let out = iii_cloud::values::skip_problem(&h.ctx, skip("LOCAL_CONFIG_NOT_SET", true))
        .await
        .unwrap();
    assert_eq!(out.codes, vec!["LOCAL_CONFIG_NOT_SET"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(config(&c), Some((Severity::Info, true)));
    assert_eq!(c.plan.warnings, 0, "a skipped warning never counts");
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert_eq!(status.skipped_problems, vec!["LOCAL_CONFIG_NOT_SET"]);
    // Undo.
    let out = iii_cloud::values::skip_problem(&h.ctx, skip("LOCAL_CONFIG_NOT_SET", false))
        .await
        .unwrap();
    assert!(out.codes.is_empty());
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(config(&c), Some((Severity::Warn, false)));
    assert!(h.ctx.store.read(|s| s.skipped_problems.is_empty()));
    // A blocking code, or any other, is refused; so is another environment.
    for code in [
        "SECRET_MISSING",
        "WEB_CONTAINER_UNKNOWN",
        "EMPTY_DEFAULT_VARIABLE",
    ] {
        let err = iii_cloud::values::skip_problem(&h.ctx, skip(code, true))
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST", "{code}");
    }
    assert!(h.ctx.store.read(|s| s.skipped_problems.is_empty()));
    let other = ProblemSkip {
        environment_id: "other".into(),
        ..skip("LOCAL_CONFIG_NOT_SET", true)
    };
    let err = iii_cloud::values::skip_problem(&h.ctx, other)
        .await
        .unwrap_err();
    assert_eq!(err.code, "ENVIRONMENT_CHANGED");
}

/// Every `git fetch` and `ls-remote` of `repo` goes through its upload-pack:
/// a script that logs one line per call counts the GitHub round trips.
fn count_round_trips(repo: &Repo) -> std::path::PathBuf {
    let log = repo.root.path().join("trips.log");
    let script = repo.root.path().join("count-upload-pack.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho x >> {}\nexec git upload-pack \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &repo.checkout,
        &[
            "config",
            "remote.origin.uploadpack",
            script.to_str().unwrap(),
        ],
    );
    log
}

fn trips(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log).map_or(0, |t| t.lines().count())
}

#[tokio::test]
async fn a_check_makes_one_round_trip_to_github_and_still_answers_the_three_questions() {
    let server = MockServer::start().await;
    let (h, repo, _ws) = ready(&server, 200).await;
    let log = count_round_trips(&repo);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(trips(&log), 1, "one fetch of the environment's branch");
    assert_eq!(c.plan.status, PlanStatus::Clean, "pushed");
    assert_eq!(c.base, git(&repo.origin, &["rev-parse", "main"]));
    assert!(
        !codes(&c).contains(&"BASE_NOT_IN_HISTORY"),
        "{:?}",
        codes(&c)
    );
    // A commit only the cloud knows is the one case with a second fetch.
    let before = trips(&log);
    check(
        &h.ctx,
        CheckRequest {
            commit: Some("abcdef1".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(trips(&log) - before, 2, "the branch and the commit");
}

#[tokio::test]
async fn the_answers_after_the_one_fetch_follow_the_remote() {
    let server = MockServer::start().await;
    let (h, repo, ws) = ready(&server, 200).await;
    let log = count_round_trips(&repo);
    // Not pushed.
    std::fs::write(ws.join("f.txt"), "f").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "local"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::NotPushed);
    assert!(!codes(&c).contains(&"BASE_NOT_IN_HISTORY"));
    // The branch moved on the remote: the new head is the base, and not an ancestor.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "c"],
    );
    let other = t.path().join("c");
    std::fs::write(other.join("x"), "x").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-m", "moved"]);
    git(&other, &["push", "-q", "origin", "main"]);
    let moved = git(&other, &["rev-parse", "HEAD"]);
    let before = trips(&log);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    // The branch's fetch, and the one more look at every branch for the unpushed commit.
    assert_eq!(trips(&log) - before, 2);
    assert_eq!(c.base, moved);
    assert!(codes(&c).contains(&"BASE_NOT_IN_HISTORY"));
}

/// A key-connected context whose `iii` reports the cloud's engine and whose
/// run ends quickly; returns the commit's workspace.
async fn with_fast_runs(h: &Harness) {
    let script = h.dir.path().join("iii");
    std::fs::write(&script, "#!/bin/sh\necho 'iii 0.27.0'\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = h.ctx.config().await;
    *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
        iii_bin: script.to_string_lossy().into(),
        data_dir: h.dir.path().join("data").to_string_lossy().into(),
        run_timeout_s: 30,
        ..(*cfg).clone()
    });
}

fn running(run: Option<&Run>) -> Run {
    run.cloned().expect("the check carries a run")
}

/// The stored run ended; make it a pass, as a real engine would have.
fn pass_stored_runs(h: &Harness) {
    h.ctx.store.update(|s| {
        for c in s.checks.iter_mut().chain(s.check.iter_mut()) {
            if let Some(r) = c.run.as_mut() {
                r.status = RunStatus::Passed;
                r.error = None;
            }
        }
    });
}

fn run_req() -> CheckRequest {
    CheckRequest {
        run: Some(true),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_passed_run_is_reused_until_something_it_was_made_from_changes() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    with_fast_runs(&h).await;
    let first = check(&h.ctx, run_req()).await.unwrap();
    let first_run = running(first.run.as_ref());
    assert!(!first_run.reused);
    assert_ne!(first_run.status, RunStatus::Passed, "the fake engine fails");
    // A failed run is never reused.
    let again = check(&h.ctx, run_req()).await.unwrap();
    assert_ne!(running(again.run.as_ref()).id, first_run.id);
    assert!(!running(again.run.as_ref()).reused);
    // Once a run passed, the same commit, file and environment answer with it.
    pass_stored_runs(&h);
    let passed = running(again.run.as_ref()).id;
    let reused = check(&h.ctx, run_req()).await.unwrap();
    let r = running(reused.run.as_ref());
    assert!(r.reused && r.id == passed && r.status == RunStatus::Passed);
    assert_eq!(r.check_id, reused.id);
    assert_eq!(r.commit, reused.commit);
    // The reuse itself survives a second call.
    assert!(running(check(&h.ctx, run_req()).await.unwrap().run.as_ref()).reused);
    // `force` runs again.
    let forced = check(
        &h.ctx,
        CheckRequest {
            force: Some(true),
            from_page: false,
            ..run_req()
        },
    )
    .await
    .unwrap();
    assert!(!running(forced.run.as_ref()).reused);
    assert_ne!(running(forced.run.as_ref()).id, passed);
    pass_stored_runs(&h);
    // A changed env value runs again; the value is nowhere in the store.
    std::fs::write(ws.join(".env"), "A=one-value-0042\n").unwrap();
    let env1 = check(&h.ctx, run_req()).await.unwrap();
    assert!(!running(env1.run.as_ref()).reused);
    pass_stored_runs(&h);
    assert!(running(check(&h.ctx, run_req()).await.unwrap().run.as_ref()).reused);
    std::fs::write(ws.join(".env"), "A=other-value-0043\n").unwrap();
    let env2 = check(&h.ctx, run_req()).await.unwrap();
    assert!(!running(env2.run.as_ref()).reused);
    let state = h.ctx.store.read(|s| serde_json::to_string(s).unwrap());
    assert!(!state.contains("one-value-0042") && !state.contains("other-value-0043"));
    pass_stored_runs(&h);
    // A compose file changed and not committed (the commit stays the same) runs again.
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "containers: {}\n# changed\n",
    )
    .unwrap();
    let compose = check(&h.ctx, run_req()).await.unwrap();
    assert!(!running(compose.run.as_ref()).reused);
    git(&ws, &["checkout", "worker-compose.yaml"]);
    pass_stored_runs(&h);
    assert!(running(check(&h.ctx, run_req()).await.unwrap().run.as_ref()).reused);
    // Another commit with the same compose file runs again.
    std::fs::write(ws.join("other.txt"), "o").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-qm", "other"]);
    let other = check(&h.ctx, run_req()).await.unwrap();
    assert!(!running(other.run.as_ref()).reused);
}

/// Each part of the reuse key moves it: the Postgres image, the engine, the
/// environment, its settings, and the compose file's content with the commit unchanged.
#[tokio::test]
async fn each_part_of_the_reuse_key_changes_it() {
    use iii_cloud::check::run_key;
    use iii_cloud::local::Manifest;
    let r = repo();
    let ws = &r.checkout;
    let skipped = std::collections::BTreeSet::new();
    let manifest = || Manifest::load(ws, Some("worker-compose.yaml")).unwrap();
    let key = |image: &'static str,
               engine: &'static str,
               env: &'static str,
               settings: serde_json::Value| {
        let m = manifest();
        let skipped = skipped.clone();
        async move {
            run_key(
                image,
                ws,
                Some(&m),
                Some(engine),
                env,
                &settings,
                &[],
                &[],
                &skipped,
            )
            .await
            .unwrap()
        }
    };
    let base = key("pg:16", "0.27.0", "env-a", json!({})).await;
    assert_eq!(base, key("pg:16", "0.27.0", "env-a", json!({})).await);
    assert_ne!(
        base,
        key("pg:17", "0.27.0", "env-a", json!({})).await,
        "image"
    );
    assert_ne!(
        base,
        key("pg:16", "0.27.1", "env-a", json!({})).await,
        "engine"
    );
    assert_ne!(
        base,
        key("pg:16", "0.27.0", "env-b", json!({})).await,
        "environment"
    );
    assert_ne!(
        base,
        key("pg:16", "0.27.0", "env-a", json!({"build_command": "x"})).await,
        "settings"
    );
    // The compose file changed and not committed: the same HEAD, another key.
    std::fs::write(ws.join("worker-compose.yaml"), "containers: {}\n# edited\n").unwrap();
    assert_ne!(
        base,
        key("pg:16", "0.27.0", "env-a", json!({})).await,
        "compose file"
    );
    // Saving a `${NAME:-}` value in iii cloud changes the key: a failed run is not served again.
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      FLAG: ${FLAG:-}\n",
    )
    .unwrap();
    let m = manifest();
    let with = |cloud_set: &'static [&'static str]| {
        let (m, skipped) = (&m, std::collections::BTreeSet::new());
        let cloud_set: Vec<String> = cloud_set.iter().map(|n| n.to_string()).collect();
        async move {
            run_key(
                "pg:16",
                ws,
                Some(m),
                Some("0.27.0"),
                "env-a",
                &json!({}),
                &[],
                &cloud_set,
                &skipped,
            )
            .await
            .unwrap()
        }
    };
    assert_ne!(with(&[]).await, with(&["FLAG"]).await, "set in iii cloud");
    assert_eq!(
        with(&[]).await,
        with(&["OTHER"]).await,
        "a name the file does not default"
    );
}

/// An `iii` whose run never gets its worker: it says who it waits on after the window.
async fn with_waiting_runs(h: &Harness, ws: &std::path::Path) {
    waiting_runs_stopping_as(h, ws, "exec sleep 20").await
}

/// The same, with the compose process's own body (a slow stop, say).
async fn waiting_runs_stopping_as(h: &Harness, ws: &std::path::Path, compose: &str) {
    let script = h.dir.path().join("iii-waiting");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ncase \"$1\" in\n  --version) echo 'iii 0.27.0';;\n  compose) {compose};;\n  call) echo '{{\"workers\":[]}}';;\nesac\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = h.ctx.config().await;
    *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
        iii_bin: script.to_string_lossy().into(),
        data_dir: h.dir.path().join("data").to_string_lossy().into(),
        run_timeout_s: 30,
        ..(*cfg).clone()
    });
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "startup_timeout: 300s\ncontainers:\n  alpha:\n    worker: package://foo\n",
    )
    .unwrap();
    git(ws, &["commit", "-qam", "a worker"]);
    // The run's own files are excluded in a real workspace.
    // (the worktree's `.git` may be a file: ask git where the file is)
    let exclude = git(
        ws,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ],
    );
    std::fs::create_dir_all(std::path::Path::new(&exclude).parent().unwrap()).unwrap();
    std::fs::write(exclude, "*.iii-cloud-run.*\n.iii\nconfig\n").unwrap();
}

#[tokio::test]
async fn a_clean_workspace_attaches_to_its_run_and_a_dirty_one_never_does() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    with_waiting_runs(&h, &ws).await;
    // Back as soon as the run says who it waits on (0.6 s under the test tick).
    let first = check(&h.ctx, run_req()).await.unwrap();
    let first_run = running(first.run.as_ref());
    assert!(first_run.waiting_on.is_some(), "{first_run:?}");
    let again = check(&h.ctx, run_req()).await.unwrap();
    assert_eq!(
        running(again.run.as_ref()).id,
        first_run.id,
        "the same key attaches"
    );
    // Uncommitted work: the key does not see it, so the call must not attach.
    std::fs::write(ws.join("dirty.txt"), "x").unwrap();
    let dirty = check(&h.ctx, run_req()).await.unwrap();
    assert_ne!(running(dirty.run.as_ref()).id, first_run.id);
    let old = h
        .ctx
        .store
        .read(|s| s.find_check(&first.id).and_then(|c| c.run.clone()))
        .unwrap();
    assert_eq!(old.error.unwrap().code, "RUN_REPLACED");
}

/// The call's deadline starts with the call: a slow plan read leaves the run's wait
/// only what is left of it (2.4 s under the test tick), not a whole new one.
#[tokio::test]
async fn a_slow_plan_read_takes_its_time_from_the_calls_deadline() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready_with(&server, 200, std::time::Duration::from_millis(2200)).await;
    with_waiting_runs(&h, &ws).await;
    // Pushed, so the cloud's plan is read (and takes its 2.2 s).
    git(&ws, &["push", "-q", "origin", "HEAD:main"]);
    let begun = std::time::Instant::now();
    let c = check(&h.ctx, run_req()).await.unwrap();
    // The run says who it waits on 0.6 s after it starts (about 2.8 s in), after the
    // deadline: the call ends at the deadline with the run still going.
    assert!(
        begun.elapsed() < std::time::Duration::from_millis(2650),
        "{:?}",
        begun.elapsed()
    );
    let run = running(c.run.as_ref());
    assert_eq!(run.status, RunStatus::Running);
    assert!(
        run.waiting_on.is_none(),
        "{:?} {:?}",
        begun.elapsed(),
        run.waiting_on
    );
}

/// A replaced run that is still stopping at the deadline is not this call's run: the
/// call fails, and no reuse key is saved for the old run.
#[tokio::test]
async fn a_replaced_run_still_stopping_at_the_deadline_gets_no_reuse_key() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    waiting_runs_stopping_as(&h, &ws, "trap 'sleep 4; exit 0' TERM; sleep 20 & wait").await;
    let first = check(&h.ctx, run_req()).await.unwrap();
    let old = running(first.run.as_ref());
    assert!(old.waiting_on.is_some());
    let key_before = h.ctx.store.read(|s| s.run_keys.get(&old.id).cloned());
    assert!(key_before.is_some());
    // Another value: another key, so the old run is replaced, and it takes 4 s to stop.
    std::fs::write(ws.join(".env"), "A=changed-value-0099\n").unwrap();
    let err = check(&h.ctx, run_req()).await.unwrap_err();
    assert_eq!(err.code, "RUN_TIMEOUT");
    assert_eq!(
        h.ctx.store.read(|s| s.run_keys.get(&old.id).cloned()),
        key_before,
        "the new key was saved for the old run"
    );
}

/// A `use` that lands while a check is in flight does not make the check of the new source.
#[tokio::test]
async fn a_check_is_recorded_for_the_source_it_started_on() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(600))
                .set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": []}})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    let ctx = h.ctx.clone();
    let running = tokio::spawn(async move { check(&ctx, CheckRequest::default()).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // The `use` of another source lands now.
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .workspace
            .as_mut()
            .unwrap()
            .source = Some("feat-b".into())
    });
    let c = running.await.unwrap().unwrap();
    let made_for = h
        .ctx
        .store
        .read(|s| s.check_sources.get(&c.id).cloned())
        .unwrap();
    assert_eq!(made_for.source, None);
}

/// What a check is recorded for is the compose file the cloud has, not the cache.
#[tokio::test]
async fn a_check_records_the_cloud_s_compose_file_not_the_cached_one() {
    let server = MockServer::start().await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "z-compose.yaml", "url": null, "settings": {}})))
        .with_priority(1)
        .mount(&server)
        .await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let made_for = h
        .ctx
        .store
        .read(|s| s.check_sources.get(&c.id).cloned())
        .unwrap();
    assert_eq!(made_for.compose.as_deref(), Some("z-compose.yaml"));
}

/// A hand-off's check made while another environment is selected still records the cloud's compose file.
#[tokio::test]
async fn a_hand_offs_check_of_another_environment_still_records_its_compose_file() {
    let server = MockServer::start().await;
    let (h, _repo, _ws) = ready(&server, 200).await;
    let first = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let created = iii_cloud::handoff::create(
        &h.ctx,
        HandoffRequest {
            check_id: Some(first.id.clone()),
            kind: HandoffKind::Ready,
            deployment_id: None,
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    // The page now shows another environment.
    h.ctx.store.update(|s| {
        s.project.as_mut().unwrap().environment.as_mut().unwrap().id = "other-env".into();
    });
    let c = check(
        &h.ctx,
        CheckRequest {
            handoff_id: Some(created.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let rec = h
        .ctx
        .store
        .read(|s| s.check_sources.get(&c.id).cloned())
        .expect("a record");
    assert_eq!(rec.compose.as_deref(), Some("worker-compose.yaml"));
    assert!(!rec.workspace);
}

#[tokio::test]
async fn a_check_that_asks_for_no_run_takes_over_the_passed_run_and_never_starts_one() {
    let server = MockServer::start().await;
    let (h, _repo, ws) = ready(&server, 200).await;
    with_fast_runs(&h).await;
    // No passed run yet: a plain check carries none, and starts none.
    let plain = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(plain.run.is_none());
    check(&h.ctx, run_req()).await.unwrap();
    pass_stored_runs(&h);
    let id = h.ctx.store.read(|s| {
        s.checks
            .iter()
            .find_map(|c| c.run.as_ref().map(|r| r.id.clone()))
            .unwrap()
    });
    // The worker's fast check after a turn: same commit and inputs, no run asked.
    let fast = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let r = running(fast.run.as_ref());
    assert!(r.reused && r.id == id && r.status == RunStatus::Passed);
    assert_eq!(r.check_id, fast.id);
    // A changed folder value: nothing is attached.
    std::fs::write(ws.join(".env"), "A=a-new-value-0042\n").unwrap();
    assert!(
        check(&h.ctx, CheckRequest::default())
            .await
            .unwrap()
            .run
            .is_none()
    );
    std::fs::remove_file(ws.join(".env")).unwrap();
    // A dirty tree: nothing is attached.
    std::fs::write(ws.join("scratch.txt"), "x").unwrap();
    assert!(
        check(&h.ctx, CheckRequest::default())
            .await
            .unwrap()
            .run
            .is_none()
    );
    std::fs::remove_file(ws.join("scratch.txt")).unwrap();
    assert!(
        check(&h.ctx, CheckRequest::default())
            .await
            .unwrap()
            .run
            .is_some()
    );
}
