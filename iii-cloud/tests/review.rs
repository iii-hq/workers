//! What the adversarial reviews found: agent tools follow their own hand-off,
//! old-attempt commits, a missing credential, an empty repository, a busy run,
//! pruning, retiring, the device sign-in after a restart, and small contracts.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use iii_cloud::check::check;
use iii_cloud::handoff::{create, retired};
use iii_cloud::project::use_project;
use iii_cloud::types::*;
use serde_json::{Value, json};
use support::Harness;
use support::repo::*;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plan_part() -> PlanPart {
    iii_cloud::check::plan_part(&json!({"can_deploy": true, "plan": {}})).0
}

fn a_check(id: &str, commit: &str, run: Option<Run>) -> Check {
    Check {
        id: id.into(),
        environment_id: ENV.into(),
        commit: commit.into(),
        base: String::new(),
        checked_at: "t".into(),
        duration_ms: 0,
        plan: plan_part(),
        here: HerePart {
            problems: 0,
            iii_version: None,
            accepts_cloud_block: true,
        },
        problems: vec![],
        run,
        ready: false,
        report: String::new(),
        manifest: None,
        handoff_id: None,
        commits: vec![],
    }
}

async fn setup() -> (Harness, MockServer, Repo, PathBuf) {
    let server = MockServer::start().await;
    let repo = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    Mock::given(method("GET"))
        .and(path("/api/v1/support-matrix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"engine": "0.27.0", "minimum_sdk": "0.26"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path_regex(r"/plan$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))).mount(&server).await;
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
    h.ctx
        .store
        .update(|s| s.put_check(a_check("chk", "abc", None)));
    (h, server, repo, ws)
}

fn ready_req() -> HandoffRequest {
    HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::Ready,
        deployment_id: None,
        text: None,
        candidate: None,
    }
}

fn commit_at(ws: &std::path::Path, file: &str, date: &str) {
    std::fs::write(ws.join(file), file).unwrap();
    git(ws, &["add", "."]);
    let out = std::process::Command::new("git")
        .args(["commit", "-qm", file])
        .current_dir(ws)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.test")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[tokio::test]
async fn an_agent_tool_follows_its_own_handoff_and_refuses_another_branch() {
    let (h, _s, _r, ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let with = |id: &str| CheckRequest {
        handoff_id: Some(id.into()),
        ..Default::default()
    };
    // The hand-off's own worktree is checked.
    let c = check(&h.ctx, with(&a.id)).await.unwrap();
    assert_eq!(c.commit, git(&ws, &["rev-parse", "HEAD"]));
    // Someone switched the worktree to another branch: refused, for every tool.
    git(&ws, &["checkout", "-q", "-b", "elsewhere"]);
    assert_eq!(
        check(&h.ctx, with(&a.id)).await.unwrap_err().code,
        "WRONG_WORKTREE"
    );
    assert_eq!(
        iii_cloud::watch::watch(
            &h.ctx,
            WatchRequest {
                handoff_id: Some(a.id.clone()),
                deployment_id: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "WRONG_WORKTREE"
    );
    assert_eq!(
        check(&h.ctx, with("no-such")).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn progress_is_bound_to_the_callers_handoff() {
    let (h, _s, _r, _ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let ok = iii_cloud::handoff::progress(
        &h.ctx,
        HandoffProgressRequest {
            handoff_id: Some(a.id.clone()),
            working_on: vec![],
            note: Some("on it".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(ok.note.as_deref(), Some("on it"));
    let err = iii_cloud::handoff::progress(
        &h.ctx,
        HandoffProgressRequest {
            handoff_id: Some("another".into()),
            working_on: vec![],
            note: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
}

#[tokio::test]
async fn commits_from_an_earlier_try_block_ready_and_the_brief_names_the_base() {
    let (h, _s, repo, ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let base = git(&repo.origin, &["rev-parse", "main"]);
    let brief = |h: &Harness| {
        iii_cloud::handoff::brief(
            &h.ctx.store.read(|s| s.clone()),
            &h.ctx.store.read(|s| s.handoffs[0].clone()),
            &[],
        )
    };
    assert_eq!(
        brief(&h).base,
        Some(BriefBase {
            branch: "main".into(),
            sha: base.clone()
        })
    );
    // A commit made before this worktree started, not on the base.
    commit_at(&ws, "old.txt", "2020-01-01T00:00:00Z");
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "OLD_ATTEMPT_COMMITS")
        .expect("an earlier try's commit is a problem");
    assert_eq!((p.owner, p.severity), (Owner::Agent, Severity::Block));
    assert!(p.title.contains("earlier try"));
    assert!(
        p.detail
            .as_deref()
            .unwrap()
            .contains(&format!("git reset --hard main ({base})"))
    );
    assert!(!c.ready);
    // Start from the base and redo it: a commit made now is this try's.
    git(&ws, &["reset", "--hard", &base]);
    commit_at(&ws, "new.txt", &chrono::Utc::now().to_rfc3339());
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(
        !c.problems.iter().any(|p| p.code == "OLD_ATTEMPT_COMMITS"),
        "{:?}",
        c.problems.iter().map(|p| &p.code).collect::<Vec<_>>()
    );
    let _ = a;
}

#[tokio::test]
async fn without_a_credential_check_still_runs_here_and_says_to_sign_in_once() {
    let (h, _s, _r, _ws) = setup().await;
    *h.ctx.cred.lock().unwrap() = None;
    h.fake.secrets.lock().unwrap().clear();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert_eq!(c.plan.status, PlanStatus::Unavailable);
    let signin: Vec<_> = c
        .problems
        .iter()
        .filter(|p| p.code == "SIGN_IN_REQUIRED")
        .collect();
    assert_eq!(signin.len(), 1);
    assert_eq!(signin[0].owner, Owner::Person);
    assert!(!c.ready);
}

#[tokio::test]
async fn an_empty_repository_is_told_to_commit_first() {
    let server = MockServer::start().await;
    let h = connected(&server, "worker-compose.yaml").await;
    let ws = h.dir.path().join("empty");
    std::fs::create_dir_all(&ws).unwrap();
    git(&ws, &["init", "-q", "-b", "main"]);
    let env = EnvironmentInfo {
        id: ENV.into(),
        name: "production".into(),
        production: true,
        branch: "main".into(),
        manifest: Some("worker-compose.yaml".into()),
        url: None,
    };
    h.ctx.store.update(|s| {
        s.project = Some(Project {
            path: ws.to_string_lossy().into(),
            repository: "ada/empty".into(),
            checkout: Checkout {
                branch: Some("main".into()),
                head: String::new(),
                changed_files: vec![],
                ahead: 0,
                not_served: None,
                branches: vec![],
            },
            team: None,
            teams: vec![],
            cloud_project: None,
            environment: Some(env),
            environments: vec![],
            importable: None,
            github_access: None,
            cloud_projects: vec![],
            other_cloud_projects: vec![],
            default_branch: None,
            workspace: Some(Workspace {
                mode: WorkspaceMode::Checkout,
                path: ws.to_string_lossy().into(),
                branch: "main".into(),
                from_head: None,
                source: None,
                source_sha: None,
            }),
            fell_back_from: None,
        });
    });
    let err = check(&h.ctx, CheckRequest::default()).await.unwrap_err();
    assert_eq!(err.code, "NO_COMMITS");
    assert!(err.message.contains("commit first"));
}

#[tokio::test]
async fn a_check_something_still_points_at_is_never_pruned() {
    let (h, _s, _r, _ws) = setup().await;
    // The oldest check has a pending merge; 25 newer ones follow.
    h.ctx.store.update(|s| {
        s.put_check_for(a_check("old-merge", "c0", None), false);
        s.merges.push(iii_cloud::store::PendingMerge {
            check_id: "old-merge".into(),
            pr: 3,
            target: "t".into(),
            merged_at: None,
        });
        for i in 1..=25 {
            s.put_check_for(a_check(&format!("c{i}"), &format!("sha{i}"), None), false);
        }
    });
    h.ctx
        .store
        .read(|s| assert!(s.find_check("old-merge").is_some()));
    h.ctx.store.read(|s| {
        assert!(s.find_check("c1").is_none(), "an unreferenced old one goes");
        assert!(s.checks.len() <= 21 + 1);
    });
}

#[tokio::test]
async fn a_done_handoff_is_retired_after_30_minutes_idle_and_the_record_stays() {
    let (h, _s, _r, _ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let get = |h: &Harness| h.ctx.store.read(|s| s.handoffs[0].clone());
    // Working is never retired, however old.
    assert!(!retired(&h.ctx, &get(&h)));
    h.ctx.store.update(|s| {
        s.handoffs[0].agent.state = AgentPhase::Done;
        s.handoffs[0].agent.last_activity_at = Some(chrono::Utc::now().to_rfc3339());
    });
    assert!(!retired(&h.ctx, &get(&h)), "just now");
    // A test second is 10 ms: 30 minutes is 18 s. Activity 20 s ago.
    let long_ago = chrono::Utc::now() - chrono::Duration::seconds(20);
    h.ctx.store.update(|s| {
        s.handoffs[0].agent.last_activity_at = Some(long_ago.to_rfc3339());
    });
    assert!(retired(&h.ctx, &get(&h)));
    assert_eq!(get(&h).id, a.id, "the record stays");
}

#[tokio::test]
async fn a_waiting_sign_in_resumes_after_a_restart_or_expires() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"code": "AUTHORIZATION_PENDING", "message": "wait"})),
        )
        .mount(&server)
        .await;
    let h = support::harness(&server.uri(), std::sync::Arc::new(support::Fake::default()));
    let waiting = |expires: chrono::DateTime<chrono::Utc>| {
        let mut conn = Connection::signed_out(&server.uri());
        conn.status = ConnStatus::Waiting;
        conn.method = Some(Method::Browser);
        conn.device = Some(Device {
            user_code: "ABCD-1234".into(),
            verification_uri: "http://x".into(),
            expires_at: expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        });
        conn
    };
    // Still valid, and its code was kept: polling resumes.
    h.ctx.store.update(|s| {
        s.connection = Some(waiting(chrono::Utc::now() + chrono::Duration::seconds(600)));
        s.pending_device = Some(("dev-code".into(), 1));
    });
    iii_cloud::connect::resume(&h.ctx);
    let mut polled = false;
    for _ in 0..100 {
        if server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/auth/device/token"))
        {
            polled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(polled, "the poll went on after the restart");
    assert_eq!(
        h.ctx.store.read(|s| s.connection.clone()).unwrap().status,
        ConnStatus::Waiting
    );
    if let Some(task) = h.ctx.poller.lock().unwrap().take() {
        task.abort();
    }
    // Past its expiry: it ends as an expired code.
    h.ctx.store.update(|s| {
        s.connection = Some(waiting(chrono::Utc::now() - chrono::Duration::seconds(5)));
    });
    iii_cloud::connect::resume(&h.ctx);
    let conn = h.ctx.store.read(|s| s.connection.clone()).unwrap();
    assert_eq!(conn.status, ConnStatus::Error);
    assert_eq!(conn.error.unwrap().code, "DEVICE_CODE_EXPIRED");
}

#[test]
fn concurrent_env_writes_all_land_and_use_the_dot_env_tmp_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".env");
    let threads: Vec<_> = (0..16)
        .map(|i| {
            let file = file.clone();
            std::thread::spawn(move || {
                iii_cloud::secret::set_env_line(&file, &format!("K{i}"), &format!("v{i}")).unwrap()
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let text = std::fs::read_to_string(&file).unwrap();
    for i in 0..16 {
        assert!(text.contains(&format!("K{i}=v{i}\n")), "{text}");
    }
    assert!(!dir.path().join(".env.tmp").exists());
    assert!(!dir.path().join(".env.env.tmp").exists());
}

#[tokio::test]
async fn sse_events_read_line_by_line_and_accept_spaced_json() {
    let server = MockServer::start().await;
    let h = connected(&server, "worker-compose.yaml").await;
    let body = "event: x\ndata: {\"type\": \"line\", \"message\": \"one\"}\n\ndata:{\"type\":\"line\",\"message\":\"two\"}\n\n: comment\ndata: {\"type\": \"ping\"}\n\ndata: {\"type\":\"line\",\"message\":\"after\"}\n\n";
    Mock::given(method("GET"))
        .and(path("/api/v1/stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    let events: Vec<Value> = h
        .ctx
        .sse_events("/stream", Duration::from_secs(5))
        .await
        .unwrap();
    let messages: Vec<&str> = events
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect();
    assert_eq!(messages, vec!["one", "two"], "the ping ends the read");
}

#[tokio::test]
async fn refresh_fills_the_default_branch_and_deleting_environments_are_dropped() {
    let (h, server, _r, _ws) = setup().await;
    h.ctx
        .store
        .update(|s| s.project.as_mut().unwrap().default_branch = None);
    iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.project.as_ref().unwrap().default_branch.clone()),
        Some("main".into())
    );
    Mock::given(method("GET"))
        .and(path("/api/v1/teams/x/projects/p/environments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"environments": [
            {"id": "e1", "name": "dev", "branch": "dev", "production": false},
            {"id": "e2", "name": "staging", "branch": "s", "production": false, "status": "deleting"}
        ]})))
        .mount(&server)
        .await;
    let envs = iii_cloud::project::read_environments(&h.ctx, "x", "p")
        .await
        .unwrap();
    assert_eq!(
        envs.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["dev"]
    );
}

#[tokio::test]
async fn a_commit_an_older_worker_made_of_copied_files_is_a_problem_before_any_deploy() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    std::fs::write(ws.join("copied.txt"), "from the person's folder").unwrap();
    git(&ws, &["add", "."]);
    git(
        &ws,
        &["commit", "-q", "-m", "Uncommitted changes from feat/search"],
    );
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "BROUGHT_FILES_COMMIT")
        .expect("the brought-files commit is a problem");
    assert_eq!((p.owner, p.severity), (Owner::Agent, Severity::Block));
    let detail = p.detail.as_deref().unwrap();
    assert!(
        detail.contains("Uncommitted changes from feat/search")
            && detail.contains("git rebase --onto")
            && !detail.contains("reset --hard"),
        "{detail}"
    );
    assert!(!c.ready);
}

#[tokio::test]
async fn files_uncommitted_in_the_folder_and_the_worktree_alike_are_copies_not_work_to_commit() {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    // The person's folder has an untracked file and a change to a tracked one.
    std::fs::write(repo.checkout.join("notes.txt"), "mine\n").unwrap();
    std::fs::write(repo.checkout.join("README.md"), "changed in the folder\n").unwrap();
    // An older worker copied them into the worktree, uncommitted.
    std::fs::write(ws.join("notes.txt"), "mine\n").unwrap();
    std::fs::write(ws.join("README.md"), "changed in the folder\n").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "BROUGHT_FILES")
        .expect("the copies are named");
    assert_eq!((p.owner, p.severity), (Owner::Agent, Severity::Block));
    let detail = p.detail.as_deref().unwrap();
    assert!(
        detail.contains("notes.txt")
            && detail.contains("Do not commit them")
            && detail.contains("git checkout --"),
        "{detail}"
    );
    assert!(!c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
    // The agent's own edit of the same file (other content) is work: commit it.
    std::fs::write(ws.join("README.md"), "the agent's own edit\n").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
    assert!(!c.problems.iter().any(|p| p.code == "BROUGHT_FILES"));
}

#[tokio::test]
async fn a_new_directory_beside_the_folders_untracked_directory_of_the_same_name_is_the_agents_work()
 {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    // The agent's work: a new directory. The person has an untracked directory of the same name, other files.
    std::fs::create_dir_all(ws.join("migrations")).unwrap();
    std::fs::write(ws.join("migrations/001.sql"), "agent\n").unwrap();
    std::fs::create_dir_all(repo.checkout.join("migrations")).unwrap();
    std::fs::write(repo.checkout.join("migrations/002.sql"), "person\n").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
    assert!(!c.problems.iter().any(|p| p.code == "BROUGHT_FILES"));
    // A file removed in both places is not a copy either.
    std::fs::remove_dir_all(ws.join("migrations")).unwrap();
    std::fs::remove_dir_all(repo.checkout.join("migrations")).unwrap();
    std::fs::remove_file(ws.join("README.md")).unwrap();
    std::fs::remove_file(repo.checkout.join("README.md")).unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!c.problems.iter().any(|p| p.code == "BROUGHT_FILES"));
}

/// A local run of the worktree left these dirty: a tracked file rewritten, a new directory of data.
async fn a_run_left_files(ws: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let before = iii_cloud::run::dirty_snapshot(ws).await.unwrap();
    std::fs::write(ws.join("README.md"), "normalized by the engine\n").unwrap();
    std::fs::create_dir_all(ws.join("api/data")).unwrap();
    std::fs::write(ws.join("api/data/a.db"), "data").unwrap();
    std::fs::write(ws.join("api/data/b.db"), "data").unwrap();
    before
}

#[tokio::test]
async fn what_a_local_run_left_in_the_worktree_is_not_the_agents_uncommitted_change() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let before = a_run_left_files(&ws).await;
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(
        !c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"),
        "{:?}",
        c.problems.iter().map(|p| &p.code).collect::<Vec<_>>()
    );
    // The agent edits one of those files afterwards: it counts again, by name, with no `git add -A`.
    std::fs::write(ws.join("README.md"), "the agent's own edit\n").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "UNCOMMITTED_CHANGES")
        .expect("an edit after the run counts");
    let detail = p.detail.as_deref().unwrap();
    assert!(
        detail.starts_with("README.md. ")
            && detail.contains("git add <paths>")
            && !detail.contains("git add -A"),
        "{detail}"
    );
    // Nothing was deleted or restored.
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "the agent's own edit\n"
    );
    assert!(ws.join("api/data/a.db").exists());
}

#[tokio::test]
async fn a_file_the_agent_changed_before_the_run_still_counts() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    std::fs::write(ws.join("mine.txt"), "uncommitted before the run\n").unwrap();
    let before = a_run_left_files(&ws).await;
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "UNCOMMITTED_CHANGES")
        .expect("the agent's own file counts");
    assert!(p.detail.as_deref().unwrap().starts_with("mine.txt. "));
}

#[tokio::test]
async fn more_than_five_hundred_files_from_a_run_are_not_set_aside() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::create_dir_all(ws.join("data")).unwrap();
    for i in 0..501 {
        std::fs::write(ws.join(format!("data/{i}.db")), "x").unwrap();
    }
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert!(
        h.ctx
            .store
            .read(|s| s.run_wrote.values().all(|w| w.files.is_empty()))
    );
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
}

#[tokio::test]
async fn a_second_run_keeps_what_the_first_left_and_a_run_that_writes_nothing_drops_nothing() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let counts = |h: &Harness| {
        h.ctx
            .store
            .read(|s| s.run_wrote.values().map(|w| w.files.len()).sum::<usize>())
    };
    let before = a_run_left_files(&ws).await;
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert_eq!(counts(&h), 3);
    // A second run rewrites the same bytes: the paths were dirty before it, with what the run left.
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::write(ws.join("README.md"), "normalized by the engine\n").unwrap();
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert_eq!(counts(&h), 3);
    // A run that writes nothing (failed before the engine, stopped, another environment's).
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert_eq!(counts(&h), 3);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
}

#[tokio::test]
async fn the_agents_uncommitted_edit_that_a_run_then_rewrites_still_counts() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    std::fs::write(ws.join("README.md"), "the agent's edit\n").unwrap();
    // The run starts with that edit in the worktree and rewrites the file.
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::write(ws.join("README.md"), "the engine's version of it\n").unwrap();
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "UNCOMMITTED_CHANGES")
        .expect("it was dirty before the run, and not the run's");
    assert!(p.detail.as_deref().unwrap().starts_with("README.md. "));
}

#[tokio::test]
async fn the_check_names_the_files_a_run_left_in_one_info_line() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let before = a_run_left_files(&ws).await;
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "RUN_LEFT_FILES")
        .expect("the set-aside files are named");
    assert_eq!((p.owner, p.severity), (Owner::None, Severity::Info));
    let d = p.detail.as_deref().unwrap();
    let blocking_here = c
        .problems
        .iter()
        .filter(|p| p.source == Source::CheckHere && p.severity == Severity::Block)
        .count() as u32;
    assert_eq!(
        c.here.problems, blocking_here,
        "a note is not a problem to count"
    );
    assert!(
        d.contains("3 files")
            && d.contains("README.md")
            && d.contains("If one of them is your own edit, commit it."),
        "{d}"
    );
    // None set aside, none named.
    std::fs::remove_dir_all(ws.join("api/data")).unwrap();
    std::fs::remove_file(ws.join("README.md")).ok();
    git(&ws, &["checkout", "--", "README.md"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!c.problems.iter().any(|p| p.code == "RUN_LEFT_FILES"));
}

#[tokio::test]
async fn after_a_restart_mid_run_the_paths_are_not_called_the_agents_own() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    std::fs::write(ws.join("README.md"), "left by the interrupted run\n").unwrap();
    // A run was going when the worker died: it is failed on the next start.
    h.ctx.store.update(|s| {
        let mut c = s.check.clone().unwrap();
        let run = Run {
            id: "r".into(),
            check_id: c.id.clone(),
            commit: c.commit.clone(),
            engine: "0.27.0".into(),
            status: RunStatus::Running,
            started_at: "t".into(),
            finished_at: None,
            stages: vec![],
            workers: WorkerCount { ready: 0, total: 1 },
            routes: RouteCount {
                count: 0,
                prefix: None,
            },
            healthcheck: None,
            not_covered: vec![],
            log_tail: vec![],
            error: None,
            reused: false,
            waiting_on: None,
            restored: vec![],
        };
        c.run = Some(run);
        s.put_check(c);
    });
    iii_cloud::run::recover(&h.ctx).await;
    let detail = |c: &Check| {
        c.problems
            .iter()
            .find(|p| p.code == "UNCOMMITTED_CHANGES")
            .and_then(|p| p.detail.clone())
    };
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let d = detail(&c).unwrap();
    assert!(
        d.contains("Some of these may be what the interrupted local run wrote. Restore only the files you did not edit, one by one;")
            && !d.contains("checkout -- ."),
        "{d}"
    );
    // The mark holds while the worktree stays dirty, and a later check still says it.
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(detail(&c).unwrap().contains("interrupted local run"));
    // Clean again: the mark is gone, and the next dirt is the agent's own.
    git(&ws, &["checkout", "--", "README.md"]);
    check(&h.ctx, CheckRequest::default()).await.unwrap();
    std::fs::write(ws.join("README.md"), "mine\n").unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!detail(&c).unwrap().contains("interrupted"));
}

#[tokio::test]
async fn a_run_over_five_hundred_files_says_so_and_a_big_file_is_compared_by_size_and_time() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::create_dir_all(ws.join("data")).unwrap();
    for i in 0..501 {
        std::fs::write(ws.join(format!("data/{i}.db")), "x").unwrap();
    }
    let note = iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert!(note.is_some_and(|n| n.contains("more than 500")));
    // A file over 4 MiB: set aside by size and time, and counted again once it is rewritten.
    std::fs::remove_dir_all(ws.join("data")).unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::write(ws.join("big.bin"), vec![1u8; 5 * 1024 * 1024]).unwrap();
    assert!(
        iii_cloud::run::set_aside(&h.ctx, &ws, &before)
            .await
            .is_none()
    );
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
    std::fs::write(ws.join("big.bin"), vec![2u8; 6 * 1024 * 1024]).unwrap();
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
}

#[tokio::test]
async fn the_record_of_a_worktree_that_is_gone_is_dropped() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    h.ctx.store.update(|s| {
        s.run_wrote
            .insert("/no/such/worktree".into(), Default::default());
    });
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    assert!(
        h.ctx
            .store
            .read(|s| !s.run_wrote.contains_key("/no/such/worktree"))
    );
}

#[tokio::test]
async fn in_the_persons_own_folder_nothing_a_run_overlaps_is_set_aside() {
    let (h, _s, repo, _ws) = setup().await;
    // Checkout mode: the workspace is the person's folder.
    let folder = repo.checkout.clone();
    h.ctx.store.update(|s| {
        s.project.as_mut().unwrap().path = folder.to_string_lossy().into_owned();
    });
    let before = iii_cloud::run::dirty_snapshot(&folder).await.unwrap();
    // The person edits a file while the run goes.
    std::fs::write(folder.join("README.md"), "typed during the run\n").unwrap();
    assert!(
        iii_cloud::run::set_aside(&h.ctx, &folder, &before)
            .await
            .is_none()
    );
    assert!(h.ctx.store.read(|s| s.run_wrote.is_empty()));
    let dirty = iii_cloud::run::dirty_split(&h.ctx, &folder).await.unwrap();
    assert_eq!(dirty.0, vec!["README.md".to_string()]);
    assert!(dirty.1.is_empty());
}

#[tokio::test]
async fn the_rebase_advice_says_to_restore_the_runs_files_first_when_a_run_left_some() {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let before = a_run_left_files(&ws).await;
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    // The base moves on.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "o").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", "HEAD:main"]);
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let p = c
        .problems
        .iter()
        .find(|p| p.code == "BASE_NOT_IN_HISTORY")
        .expect("the base moved");
    assert!(
        p.detail
            .as_deref()
            .unwrap()
            .ends_with("Restore the local run's files first.")
    );
}

/// Two tracked config files, committed, as the engine's seed files are.
fn commit_seed_files(ws: &std::path::Path) {
    std::fs::create_dir_all(ws.join("config")).unwrap();
    std::fs::write(ws.join("config/http.yaml"), "# comment\nport: ${PORT}\n").unwrap();
    std::fs::write(ws.join("config/queue.yaml"), "# queue\nname: q\n").unwrap();
    git(ws, &["add", "."]);
    git(ws, &["commit", "-q", "-m", "seed files"]);
}

#[tokio::test]
async fn the_tracked_files_a_run_rewrote_are_put_back_and_kept_aside() {
    use std::os::unix::fs::PermissionsExt;
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    commit_seed_files(&ws);
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    // The engine normalizes the two files and the workers write data.
    std::fs::write(ws.join("config/http.yaml"), "port: 8080\n").unwrap();
    std::fs::write(ws.join("config/queue.yaml"), "name: q\n").unwrap();
    std::fs::create_dir_all(ws.join("api/data")).unwrap();
    std::fs::write(ws.join("api/data/a.db"), "data").unwrap();
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    assert_eq!(done.restored, vec!["config/http.yaml", "config/queue.yaml"]);
    // Only the new directory is left, and git says so.
    assert_eq!(git(&ws, &["status", "--porcelain"]), "?? api/");
    assert_eq!(
        std::fs::read_to_string(ws.join("config/http.yaml")).unwrap(),
        "# comment\nport: ${PORT}\n"
    );
    // The rewritten versions are kept, private.
    let kept = run_dir.path().join("left/config/http.yaml");
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "port: 8080\n");
    assert_eq!(
        std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(run_dir.path().join("left/config"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // The next check: nothing uncommitted, one info about the restore, one about what is left.
    let c = check(&h.ctx, CheckRequest::default()).await.unwrap();
    assert!(!c.problems.iter().any(|p| p.code == "UNCOMMITTED_CHANGES"));
    let restored: Vec<_> = c
        .problems
        .iter()
        .filter(|p| p.code == "RUN_FILES_RESTORED")
        .collect();
    assert_eq!(restored.len(), 1);
    let (title, detail) = (&restored[0].title, restored[0].detail.as_deref().unwrap());
    assert_eq!(
        title,
        "The local run rewrote 2 tracked files; they are back as committed"
    );
    assert!(
        detail.contains("config/http.yaml")
            && detail.contains(&run_dir.path().join("left").display().to_string())
    );
    assert!(
        !detail.contains("port: 8080"),
        "never a kept file's content"
    );
    assert!(
        detail.ends_with(&format!(
            "Compare with `diff {}/<path> <path>`.",
            run_dir.path().join("left").display()
        )),
        "{detail}"
    );
    let left = c
        .problems
        .iter()
        .find(|p| p.code == "RUN_LEFT_FILES")
        .unwrap();
    assert!(
        left.detail.as_deref().unwrap().contains("api/data/a.db")
            && !left.detail.as_deref().unwrap().contains("config/")
    );
}

#[tokio::test]
async fn a_file_dirty_before_the_run_is_not_put_back_and_a_deletion_is() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    commit_seed_files(&ws);
    std::fs::write(ws.join("config/queue.yaml"), "the agent's edit\n").unwrap();
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::write(ws.join("config/queue.yaml"), "rewritten by the run\n").unwrap();
    std::fs::remove_file(ws.join("config/http.yaml")).unwrap();
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    assert_eq!(done.restored, vec!["config/http.yaml"]);
    assert!(
        ws.join("config/http.yaml").exists(),
        "the deletion is undone"
    );
    assert!(
        !run_dir.path().join("left/config/http.yaml").exists(),
        "nothing to keep of a deletion"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("config/queue.yaml")).unwrap(),
        "rewritten by the run\n",
        "a file that was dirty before the run is never touched"
    );
}

#[tokio::test]
async fn in_the_persons_own_folder_nothing_is_put_back_and_a_big_file_stays_set_aside() {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    commit_seed_files(&ws);
    // The person's folder: untouched.
    let folder = repo.checkout.clone();
    h.ctx.store.update(|s| {
        s.project.as_mut().unwrap().path = folder.to_string_lossy().into_owned();
    });
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&folder).await.unwrap();
    std::fs::write(folder.join("README.md"), "typed by the person\n").unwrap();
    let done = iii_cloud::run::set_aside_run(&h.ctx, &folder, &before, Some(run_dir.path())).await;
    assert!(done.restored.is_empty());
    assert_eq!(
        std::fs::read_to_string(folder.join("README.md")).unwrap(),
        "typed by the person\n"
    );
    // A worktree: a tracked file over 4 MiB is left where it is, and set aside.
    h.ctx.store.update(|s| {
        s.project.as_mut().unwrap().path = "/the/persons/own/folder".into();
    });
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::write(ws.join("config/http.yaml"), vec![b'x'; 5 * 1024 * 1024]).unwrap();
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    assert!(done.restored.is_empty());
    assert!(h.ctx.store.read(|s| {
        s.run_wrote
            .values()
            .any(|w| w.files.contains_key("config/http.yaml"))
    }));
}

#[tokio::test]
async fn a_newer_run_does_not_take_away_what_the_older_one_kept() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    commit_seed_files(&ws);
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for (dir, text) in [(&one, "first\n"), (&two, "second\n")] {
        let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
        std::fs::write(ws.join("config/http.yaml"), text).unwrap();
        let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(dir.path())).await;
        assert_eq!(done.restored.len(), 1);
    }
    assert_eq!(
        std::fs::read_to_string(one.path().join("left/config/http.yaml")).unwrap(),
        "first\n",
        "the older run's copy stays until its folder is pruned"
    );
    assert_eq!(
        std::fs::read_to_string(two.path().join("left/config/http.yaml")).unwrap(),
        "second\n"
    );
}

#[tokio::test]
async fn the_restore_first_advice_is_for_set_aside_files_that_are_tracked_and_dirty_now() {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    // Only untracked data was set aside: a rebase is not stopped by it.
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    std::fs::create_dir_all(ws.join("api/data")).unwrap();
    std::fs::write(ws.join("api/data/a.db"), "d").unwrap();
    iii_cloud::run::set_aside(&h.ctx, &ws, &before).await;
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    std::fs::write(c.join("other.txt"), "o").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "other"]);
    git(&c, &["push", "-q", "origin", "HEAD:main"]);
    let checked = check(&h.ctx, CheckRequest::default()).await.unwrap();
    let d = checked
        .problems
        .iter()
        .find(|p| p.code == "BASE_NOT_IN_HISTORY")
        .and_then(|p| p.detail.clone())
        .unwrap();
    assert!(!d.contains("Restore the local run's files first"), "{d}");
}

#[tokio::test]
async fn a_path_with_pattern_characters_is_put_back_alone() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    // Names a glob or pathspec magic would read as more than themselves.
    let names = [
        "app/[id]/page.tsx",
        "app/i/page.tsx",
        "s*.yaml",
        "sx.yaml",
        "q?.yaml",
        "qa.yaml",
        ":colon.yaml",
        "colon.yaml",
    ];
    for n in names {
        std::fs::create_dir_all(ws.join(n).parent().unwrap()).unwrap();
        std::fs::write(ws.join(n), "committed\n").unwrap();
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "names"]);
    // The agent's own edits, made before the run: the siblings a glob would also reach.
    for n in ["app/i/page.tsx", "sx.yaml", "qa.yaml", "colon.yaml"] {
        std::fs::write(ws.join(n), "the agent's edit\n").unwrap();
    }
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    for n in ["app/[id]/page.tsx", "s*.yaml", "q?.yaml", ":colon.yaml"] {
        std::fs::write(ws.join(n), "rewritten by the run\n").unwrap();
    }
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    let mut got = done.restored.clone();
    got.sort();
    assert_eq!(
        got,
        vec![":colon.yaml", "app/[id]/page.tsx", "q?.yaml", "s*.yaml"]
    );
    for n in ["app/[id]/page.tsx", "s*.yaml", "q?.yaml", ":colon.yaml"] {
        assert_eq!(
            std::fs::read_to_string(ws.join(n)).unwrap(),
            "committed\n",
            "{n}"
        );
    }
    for n in ["app/i/page.tsx", "sx.yaml", "qa.yaml", "colon.yaml"] {
        assert_eq!(
            std::fs::read_to_string(ws.join(n)).unwrap(),
            "the agent's edit\n",
            "{n} was dirty before the run and is never touched"
        );
    }
}

#[tokio::test]
async fn one_runs_kept_copies_stop_at_64_mib_and_the_rest_stay_set_aside() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let big = |text: u8| vec![text; 4 * 1024 * 1024 - 16];
    for i in 0..18 {
        std::fs::write(ws.join(format!("big{i:02}.bin")), big(b'a')).unwrap();
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "big files"]);
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    for i in 0..18 {
        std::fs::write(ws.join(format!("big{i:02}.bin")), big(b'b')).unwrap();
    }
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    assert_eq!(done.restored.len(), 16, "64 MiB of 4 MiB files");
    let left = h
        .ctx
        .store
        .read(|s| s.run_wrote.get(&ws.to_string_lossy().into_owned()).cloned())
        .unwrap();
    assert_eq!(left.files.len(), 2, "the rest stay set aside");
}

#[tokio::test]
async fn a_file_where_a_tracked_directory_was_is_left_alone_and_a_deleted_directory_is_back() {
    let (h, _s, _repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    std::fs::create_dir_all(ws.join("p")).unwrap();
    std::fs::write(ws.join("p/a"), "tracked\n").unwrap();
    std::fs::create_dir_all(ws.join("gone")).unwrap();
    std::fs::write(ws.join("gone/b"), "tracked\n").unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "dirs"]);
    let run_dir = tempfile::tempdir().unwrap();
    let before = iii_cloud::run::dirty_snapshot(&ws).await.unwrap();
    // The directory `p` becomes a file (the run's data), and `gone/` is removed whole.
    std::fs::remove_dir_all(ws.join("p")).unwrap();
    std::fs::write(ws.join("p"), "run data\n").unwrap();
    std::fs::remove_dir_all(ws.join("gone")).unwrap();
    let done = iii_cloud::run::set_aside_run(&h.ctx, &ws, &before, Some(run_dir.path())).await;
    assert_eq!(done.restored, vec!["gone/b"]);
    assert_eq!(
        std::fs::read_to_string(ws.join("p")).unwrap(),
        "run data\n",
        "never deleted"
    );
    assert!(ws.join("gone/b").exists());
    assert!(
        h.ctx
            .store
            .read(|s| s.run_wrote.values().any(|w| w.files.contains_key("p")))
    );
}
