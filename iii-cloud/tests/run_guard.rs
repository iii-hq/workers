//! A run that cannot start or hits a bad compose file fails cleanly and frees the lock.

mod support;

use std::time::{Duration, Instant};

use iii_cloud::run::start;
use iii_cloud::types::*;
use serde_json::json;
use support::repo::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn with_check(server: &MockServer, compose: &str) -> (support::Harness, std::path::PathBuf) {
    let h = connected(server, "worker-compose.yaml").await;
    let ws = h.dir.path().join("app");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("worker-compose.yaml"), compose).unwrap();
    git(&ws, &["init", "-b", "main"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "app"]);
    let head = git(&ws, &["rev-parse", "HEAD"]);
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
            path: ws.to_string_lossy().into(), repository: "ada/app".into(),
            checkout: Checkout { branch: Some("main".into()), head: head.clone(), changed_files: vec![], ahead: 0, not_served: None, branches: vec![] },
            team: None, teams: vec![], cloud_project: None, environment: Some(env), environments: vec![], importable: None, github_access: None, cloud_projects: vec![], other_cloud_projects: vec![], default_branch: None,
            workspace: Some(Workspace { mode: WorkspaceMode::Checkout, path: ws.to_string_lossy().into(), branch: "main".into(), from_head: None, not_brought: vec![], source: None, not_brought_reasons: Default::default(), source_sha: None }),
            fell_back_from: None,
        });
        s.put_check(Check {
            id: "chk".into(), environment_id: ENV.into(), commit: head, base: String::new(), checked_at: "t".into(), duration_ms: 0,
            plan: serde_json::from_value(json!({"status": "clean", "fingerprint": null, "digest": null, "blocking": 0, "warnings": 0, "at_least": false,
                "confirmations": {"resources": [], "overrides": []}, "web_candidates": null, "error": null})).unwrap(),
            here: HerePart { problems: 0, iii_version: None, accepts_cloud_block: true }, problems: vec![], run: None, ready: false, report: String::new(), manifest: None, handoff_id: None, commits: vec![],
        });
    });
    (h, ws)
}

#[tokio::test]
async fn a_compose_file_of_the_wrong_shape_fails_the_run_and_frees_the_lock() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, "containers: {}\nengine: nope\n").await;
    start(&h.ctx, "chk".into()).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let run = loop {
        let run = h
            .ctx
            .store
            .read(|s| s.check.as_ref().and_then(|c| c.run.clone()))
            .unwrap();
        if run.status != RunStatus::Running && run.finished_at.is_some() {
            break run;
        }
        assert!(Instant::now() < deadline, "{run:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error.unwrap().code, "COMPOSE_FAILED");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.ctx.runs.lock().unwrap().is_empty(), "the slot is free");
    // And a new run may start.
    start(&h.ctx, "chk".into()).await.unwrap();
}

#[tokio::test]
async fn a_run_does_not_start_when_the_environment_settings_cannot_be_read() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, "containers: {}\n").await;
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({"code": "INTERNAL", "message": "down"})),
        )
        .mount(&server)
        .await;
    let err = start(&h.ctx, "chk".into()).await.unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert!(h.ctx.runs.lock().unwrap().is_empty());
    assert!(
        h.ctx
            .store
            .read(|s| s.check.as_ref().unwrap().run.is_none()),
        "no run was recorded"
    );
}

async fn finished(h: &support::Harness) -> Run {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let run = h
            .ctx
            .store
            .read(|s| s.check.as_ref().and_then(|c| c.run.clone()))
            .unwrap();
        if run.status != RunStatus::Running && run.finished_at.is_some() {
            return run;
        }
        assert!(Instant::now() < deadline, "{run:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

const PATH_WORKER: &str = "containers:\n  api:\n    worker: path://./workers/api\n";

#[tokio::test]
async fn a_worker_with_no_language_gets_its_runtime_from_the_plan() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, PATH_WORKER).await;
    // The worker's own iii.worker.yaml says nothing about its runtime; the plan says node@24.
    std::fs::create_dir_all(ws.join("workers/api")).unwrap();
    iii_cloud::deploy::save_plan(
        &h.ctx,
        "chk",
        &json!({"workers": [{"name": "api", "kind": "path", "runtime": "node@24", "runtime_source": "detected"}]}),
    );
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = finished(&h).await;
    let error = run.error.unwrap();
    // It reached the install (node, no lock here): not skipped as "no path:// worker".
    assert_eq!(
        (error.stage, error.code.as_str()),
        (StageName::Install, "INSTALL_FAILED")
    );
    assert!(
        error.message.contains("no lock for node"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn a_worker_whose_runtime_is_unknown_fails_the_install_with_its_name() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, PATH_WORKER).await;
    std::fs::create_dir_all(ws.join("workers/api")).unwrap();
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = finished(&h).await;
    let error = run.error.unwrap();
    assert_eq!(error.code, "INSTALL_FAILED");
    assert!(
        error.message.contains("runtime of api is not known"),
        "{}",
        error.message
    );
    let install = run
        .stages
        .iter()
        .find(|s| s.name == StageName::Install)
        .unwrap();
    assert_eq!(install.status, StageStatus::Failed, "not a silent skip");
}

#[tokio::test]
async fn the_run_uses_the_file_the_check_was_of_not_the_environments() {
    let server = MockServer::start().await;
    // The environment's file would fail the install; the checked one fails at compose.
    let (h, ws) = with_check(&server, PATH_WORKER).await;
    std::fs::create_dir_all(ws.join("workers/api")).unwrap();
    std::fs::write(
        ws.join("worker-compose-next.yaml"),
        "containers: {}\nengine: nope\n",
    )
    .unwrap();
    h.ctx.store.update(|s| {
        s.check.as_mut().unwrap().manifest = Some("worker-compose-next.yaml".into());
        s.checks[0].manifest = Some("worker-compose-next.yaml".into());
    });
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = finished(&h).await;
    assert_eq!(run.error.unwrap().code, "COMPOSE_FAILED");
}

/// What the run's `.env` lacks comes from the person's folder's env file into
/// the run's processes only: never into a log file, the run record, the state
/// or an event.
#[tokio::test]
async fn a_name_the_runs_env_lacks_comes_from_the_folders_env_file_into_the_process_only() {
    let server = MockServer::start().await;
    let compose = "containers:\n  api:\n    worker: package://http\n    environment:\n      GH_TOKEN: ${GH_TOKEN}\ncloud:\n  release:\n    worker: api\n    command: printenv GH_TOKEN > seen.txt; echo \"the token is $GH_TOKEN\"\n";
    let (h, ws) = with_check(&server, compose).await;
    // No real iii: the run stops at compose, after the release.
    *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
        cloud_url: server.uri(),
        iii_bin: "iii-not-installed-here".into(),
        data_dir: h.dir.path().join("data").to_string_lossy().into_owned(),
        ..iii_cloud::config::Config::default()
    });
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .name = "test".into();
    });
    let value = "run-secret-value-0009";
    std::fs::write(ws.join(".env.test"), format!("GH_TOKEN={value}\n")).unwrap();
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = finished(&h).await;
    let release = run
        .stages
        .iter()
        .find(|s| s.name == StageName::Release)
        .unwrap();
    assert_eq!(release.status, StageStatus::Passed, "{run:?}");
    assert_eq!(
        std::fs::read_to_string(ws.join("seen.txt")).unwrap().trim(),
        value,
        "it reached the release's environment"
    );
    let runs = h.dir.path().join("data/runs");
    let log = std::fs::read_dir(&runs)
        .unwrap()
        .flatten()
        .map(|d| std::fs::read_to_string(d.path().join("run.log")).unwrap_or_default())
        .collect::<String>();
    assert!(log.contains("the token is [redacted]"), "{log}");
    let state = h.ctx.store.read(|s| serde_json::to_string(s).unwrap());
    let events = serde_json::to_string(&*h.fake.events.lock().unwrap()).unwrap();
    for text in [&log, &format!("{:?}", run.log_tail), &state, &events] {
        assert!(!text.contains(value), "{text}");
    }
    assert!(!ws.join(".env").exists(), "never written to a file");
    assert!(
        h.ctx
            .known_secrets
            .lock()
            .unwrap()
            .contains(&value.to_string())
    );
}

/// A name skipped for now reaches the run's processes as `""`, and no file.
#[tokio::test]
async fn a_skipped_name_runs_with_an_empty_value() {
    let server = MockServer::start().await;
    let compose = "containers:\n  api:\n    worker: package://http\n    environment:\n      GH_TOKEN: ${GH_TOKEN}\ncloud:\n  release:\n    worker: api\n    command: if printenv GH_TOKEN >/dev/null; then echo \"set:$GH_TOKEN.\" > seen.txt; else echo unset > seen.txt; fi\n";
    let (h, ws) = with_check(&server, compose).await;
    *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
        cloud_url: server.uri(),
        iii_bin: "iii-not-installed-here".into(),
        data_dir: h.dir.path().join("data").to_string_lossy().into_owned(),
        ..iii_cloud::config::Config::default()
    });
    iii_cloud::values::skip(
        &h.ctx,
        ValuesSkip {
            environment_id: ENV.into(),
            names: vec!["GH_TOKEN".into()],
            skip: true,
        },
    )
    .await
    .unwrap();
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = finished(&h).await;
    assert!(
        run.stages
            .iter()
            .any(|s| s.name == StageName::Release && s.status == StageStatus::Passed),
        "{run:?}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("seen.txt")).unwrap().trim(),
        "set:."
    );
    assert!(!ws.join(".env").exists());
}

// ------------------------------------------------ one run per workspace, the workers stage

const TWO_WORKERS: &str =
    "containers:\n  alpha:\n    worker: package://foo\n  beta:\n    worker: package://bar\n";
const ALL: &str =
    r#"{"workers":[{"name":"alpha","status":"connected"},{"name":"beta","status":"connected"}]}"#;
const ONLY_ALPHA: &str = r#"{"workers":[{"name":"alpha","status":"connected"}]}"#;

/// A stand-in for `iii`: `compose` runs `compose_body` in the compose file's folder
/// and then sleeps; `call` answers from `workers.json`.
fn fake_iii(h: &support::Harness, compose_body: &str) -> std::path::PathBuf {
    let dir = h.dir.path().join("fake-iii");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("iii");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'iii 0.27.0';;\n  compose) {compose_body}\n    exec sleep 300;;\n  call) case \"$4\" in\n    engine::workers::list) cat '{w}';;\n    *) echo '{{\"observers\":[]}}';;\n  esac;;\nesac\n",
            w = dir.join("workers.json").display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_workers(h, "{\"workers\":[]}");
    bin
}

fn set_workers(h: &support::Harness, json: &str) {
    std::fs::write(h.dir.path().join("fake-iii/workers.json"), json).unwrap();
}

async fn use_iii(h: &support::Harness, server: &MockServer, bin: &std::path::Path) {
    *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
        cloud_url: server.uri(),
        iii_bin: bin.to_string_lossy().into(),
        data_dir: h.dir.path().join("data").to_string_lossy().into_owned(),
        ..iii_cloud::config::Config::default()
    });
}

fn run_of(h: &support::Harness, check: &str) -> Option<Run> {
    h.ctx
        .store
        .read(|s| s.find_check(check).and_then(|c| c.run.clone()))
}

async fn ended(h: &support::Harness, check: &str) -> Run {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(run) = run_of(h, check).filter(|r| r.finished_at.is_some()) {
            return run;
        }
        assert!(Instant::now() < deadline, "{:?}", run_of(h, check));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn selected_of(h: &support::Harness, ws: &std::path::Path) -> iii_cloud::check::Selected {
    iii_cloud::check::Selected {
        workspace: ws.to_path_buf(),
        env: h
            .ctx
            .store
            .read(|s| s.project.as_ref().unwrap().environment.clone().unwrap()),
        team_id: TEAM.into(),
    }
}

/// Another check of the same environment, on `ws`'s current HEAD.
fn add_check(h: &support::Harness, ws: &std::path::Path, id: &str) {
    let commit = git(ws, &["rev-parse", "HEAD"]);
    h.ctx.store.update(|s| {
        let mut c = s.find_check("chk").cloned().unwrap();
        c.id = id.into();
        c.commit = commit;
        c.run = None;
        s.put_check_for(c, false);
    });
}

/// A second workspace with the same compose file, committed.
fn add_workspace(h: &support::Harness, name: &str) -> std::path::PathBuf {
    let ws = h.dir.path().join(name);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("worker-compose.yaml"), TWO_WORKERS).unwrap();
    git(&ws, &["init", "-b", "main"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "app"]);
    ws
}

#[tokio::test]
async fn no_new_worker_for_a_minute_fails_early_naming_the_container_and_its_log() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(
        &h,
        "mkdir -p .iii/compose/iii-cloud-run/logs; printf '# header\\nstdout\\tlistening\\nstderr\\tboom-from-beta\\n' > .iii/compose/iii-cloud-run/logs/beta.log;",
    );
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    let begun = Instant::now();
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = ended(&h, "chk").await;
    // The 60 s window is 600 ms under the test tick; the compose file's own ceiling is 60 s.
    assert!(begun.elapsed() < Duration::from_secs(20));
    let error = run.error.unwrap();
    assert_eq!(
        (error.stage, error.code.as_str()),
        (StageName::Workers, "WORKERS_NOT_READY")
    );
    assert!(error.message.contains("no new one"), "{}", error.message);
    assert!(
        error.message.contains("Did not connect: beta."),
        "{}",
        error.message
    );
    assert!(
        run.log_tail.iter().any(|l| l == "[beta] boom-from-beta"),
        "{:?}",
        run.log_tail
    );
    assert!(!run.log_tail.iter().any(|l| l.contains("# header")));
}

#[tokio::test]
async fn a_container_that_exited_fails_the_stage_at_once() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, TWO_WORKERS).await;
    // A pid nothing has: a process that ended and was reaped.
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    let bin = fake_iii(
        &h,
        &format!(
            "mkdir -p .iii/compose/iii-cloud-run/logs; echo 'stderr\tcrashed-beta' > .iii/compose/iii-cloud-run/logs/beta.log; echo '{{\"containers\":{{\"beta\":{{\"pid\":{pid},\"status\":\"ready\"}}}}}}' > .iii/compose/iii-cloud-run/state.json;"
        ),
    );
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    start(&h.ctx, "chk".into()).await.unwrap();
    let run = ended(&h, "chk").await;
    let error = run.error.unwrap();
    assert_eq!(error.code, "WORKERS_NOT_READY");
    assert!(
        error
            .message
            .contains("beta exited before all workers connected"),
        "{}",
        error.message
    );
    assert!(run.log_tail.iter().any(|l| l == "[beta] crashed-beta"));
}

#[tokio::test]
async fn a_run_of_another_workspace_starts_while_one_is_running() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    let ws2 = add_workspace(&h, "other");
    add_check(&h, &ws2, "chk2");
    let first = start(&h.ctx, "chk".into()).await.unwrap();
    let second = iii_cloud::run::start_with(&h.ctx, "chk2".into(), selected_of(&h, &ws2))
        .await
        .expect("no RUN_BUSY for another workspace");
    assert_ne!(first.id, second.id);
    assert_eq!(h.ctx.runs.lock().unwrap().len(), 2);
    set_workers(&h, ALL);
    assert_eq!(ended(&h, "chk").await.status, RunStatus::Passed);
    assert_eq!(ended(&h, "chk2").await.status, RunStatus::Passed);
    assert!(ws.exists());
}

#[tokio::test]
async fn a_new_commit_of_the_workspace_replaces_the_running_run() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    let first = start(&h.ctx, "chk".into()).await.unwrap();
    std::fs::write(ws.join("notes.txt"), "fix").unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "fix"]);
    add_check(&h, &ws, "chk2");
    let second = start(&h.ctx, "chk2".into()).await.unwrap();
    assert_ne!(first.id, second.id);
    // The old one was torn down before the new one started.
    let old = run_of(&h, "chk").unwrap();
    assert_eq!(old.status, RunStatus::Failed);
    assert_eq!(old.error.unwrap().code, "RUN_REPLACED");
    let events = h.fake.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e.kind == EventKind::Run
            && e.id == first.id
            && e.terminal
            && e.record["error"]["code"] == "RUN_REPLACED"),
        "{events:?}"
    );
    set_workers(&h, ALL);
    assert_eq!(ended(&h, "chk2").await.status, RunStatus::Passed);
}

#[tokio::test]
async fn the_same_commit_attaches_to_the_running_run() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    // The run's own files are excluded in a real workspace; else it is `+dirty` at once.
    std::fs::write(
        ws.join(".git/info/exclude"),
        "*.iii-cloud-run.*\n.iii\nconfig\n",
    )
    .unwrap();
    let first = start(&h.ctx, "chk".into()).await.unwrap();
    let ctx = h.ctx.clone();
    let attached = tokio::spawn(async move { start(&ctx, "chk".into()).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!attached.is_finished(), "it waits for the run");
    set_workers(&h, ALL);
    let run = attached.await.unwrap().unwrap();
    assert_eq!(
        (run.id.as_str(), run.status),
        (first.id.as_str(), RunStatus::Passed)
    );
    let runs = std::fs::read_dir(h.dir.path().join("data/runs"))
        .unwrap()
        .count();
    assert_eq!(runs, 1, "no second run");
}
