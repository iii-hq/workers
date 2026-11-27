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
            workspace: Some(Workspace { mode: WorkspaceMode::Checkout, path: ws.to_string_lossy().into(), branch: "main".into(), from_head: None, source: None, source_sha: None }),
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
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'iii 0.27.0';;\n  compose) {compose_body}\n    exec sleep 20;;\n  call) case \"$4\" in\n    engine::workers::list) cat '{w}';;\n    *) echo '{{\"observers\":[]}}';;\n  esac;;\nesac\n",
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

/// Two workers, a startup timeout of 120 s (1.2 s under the test tick: the 60 s
/// window is 0.6 s).
const SLOW_START: &str = "startup_timeout: 120s\ncontainers:\n  alpha:\n    worker: package://foo\n  beta:\n    worker: package://bar\n";
const BETA_LOG: &str = "mkdir -p .iii/compose/iii-cloud-run/logs; printf '# header\\nstdout\\tlistening\\nstderr\\tboom-from-beta\\n' > .iii/compose/iii-cloud-run/logs/beta.log;";

async fn waiting(h: &support::Harness, check: &str) -> Run {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(run) = run_of(h, check).filter(|r| r.waiting_on.is_some()) {
            return run;
        }
        assert!(Instant::now() < deadline, "{:?}", run_of(h, check));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_minute_with_no_new_worker_says_who_it_waits_on_and_the_run_goes_on() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, SLOW_START).await;
    let bin = fake_iii(&h, BETA_LOG);
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    let begun = Instant::now();
    start(&h.ctx, "chk".into()).await.unwrap();
    // Before the window nothing is said; the window (0.6 s) passes, the ceiling (1.2 s) not yet.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(run_of(&h, "chk").unwrap().waiting_on.is_none());
    let run = waiting(&h, "chk").await;
    assert!(
        begun.elapsed() >= Duration::from_millis(550),
        "the window is 0.6 s"
    );
    assert_eq!(run.status, RunStatus::Running, "not a failure");
    assert_eq!(run.waiting_on.as_ref().unwrap().containers, vec!["beta"]);
    assert!(
        run.log_tail.iter().any(|l| l == "[beta] boom-from-beta"),
        "{:?}",
        run.log_tail
    );
    assert!(!run.log_tail.iter().any(|l| l.contains("# header")));
    // A non-terminal event said so.
    let events = h.fake.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|e| e.kind == EventKind::Run && !e.terminal && !e.record["waiting_on"].is_null()),
        "{events:?}"
    );
    // The compose file's ceiling still ends it, as the cloud's does.
    let run = ended(&h, "chk").await;
    let error = run.error.unwrap();
    assert_eq!(error.code, "WORKERS_NOT_READY");
    assert!(
        error.message.contains("Did not connect: beta."),
        "{}",
        error.message
    );
    assert!(run.waiting_on.is_none());
}

#[tokio::test]
async fn a_worker_that_connects_after_the_wait_clears_it_and_the_run_passes() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, SLOW_START).await;
    let bin = fake_iii(&h, BETA_LOG);
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    start(&h.ctx, "chk".into()).await.unwrap();
    waiting(&h, "chk").await;
    set_workers(&h, ALL);
    let run = ended(&h, "chk").await;
    assert_eq!(run.status, RunStatus::Passed, "{:?}", run.error);
    assert!(run.waiting_on.is_none());
}

#[tokio::test]
async fn a_container_restarting_is_not_an_exit() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, SLOW_START).await;
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    // `restart: on-failure`: the record keeps the dead pid while it backs off.
    let bin = fake_iii(
        &h,
        &format!(
            "mkdir -p .iii/compose/iii-cloud-run; echo '{{\"containers\":{{\"beta\":{{\"pid\":{pid},\"status\":\"restarting\"}}}}}}' > .iii/compose/iii-cloud-run/state.json;"
        ),
    );
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    start(&h.ctx, "chk".into()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let run = run_of(&h, "chk").unwrap();
    assert_eq!((run.status, run.error), (RunStatus::Running, None));
    set_workers(&h, ALL);
    assert_eq!(ended(&h, "chk").await.status, RunStatus::Passed);
}

#[test]
fn a_taken_postgres_port_is_not_reported_as_a_missing_docker() {
    let (code, message) = iii_cloud::run::docker_failure(
        49000,
        "docker: Error response: Bind for 127.0.0.1:49000 failed: port is already allocated.",
    );
    assert_eq!(code, "PORT_IN_USE");
    assert!(message.contains("49000"), "{message}");
    assert_eq!(
        iii_cloud::run::docker_failure(1, "no such image").0,
        "DOCKER_MISSING"
    );
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
    let second = begin(&h, "chk2", &ws2, None).await.unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(h.ctx.runs.lock().unwrap().len(), 2);
    // Each run holds its own ports until its teardown.
    assert!(h.ctx.ports.lock().unwrap().len() >= 2);
    set_workers(&h, ALL);
    assert_eq!(ended(&h, "chk").await.status, RunStatus::Passed);
    assert_eq!(ended(&h, "chk2").await.status, RunStatus::Passed);
    assert!(ws.exists());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(h.ctx.ports.lock().unwrap().is_empty(), "given back");
}

/// `start_with` on `ws` as the environment's check `id`, with a reuse key and a 20 s deadline.
async fn begin(
    h: &support::Harness,
    id: &str,
    ws: &std::path::Path,
    key: Option<&str>,
) -> Result<Run, iii_cloud::error::CloudError> {
    let until = Instant::now() + Duration::from_secs(20);
    iii_cloud::run::start_with(
        &h.ctx,
        id.into(),
        selected_of(h, ws),
        key.map(Into::into),
        until,
    )
    .await
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
async fn the_same_key_attaches_a_second_check_to_the_running_run() {
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
    add_check(&h, &ws, "chk2");
    let first = begin(&h, "chk", &ws, Some("k")).await.unwrap();
    // Another check of the same commit, environment and key.
    let (hh, ws2) = (h.ctx.clone(), ws.clone());
    let sel = selected_of(&h, &ws2);
    let attached = tokio::spawn(async move {
        let until = Instant::now() + Duration::from_secs(20);
        iii_cloud::run::start_with(&hh, "chk2".into(), sel, Some("k".into()), until).await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!attached.is_finished(), "it waits for the run");
    // The run is in the second check too, as it goes.
    assert_eq!(run_of(&h, "chk2").unwrap().id, first.id);
    set_workers(&h, ALL);
    let run = attached.await.unwrap().unwrap();
    assert_eq!(
        (run.id.as_str(), run.status),
        (first.id.as_str(), RunStatus::Passed)
    );
    // Its end reached the attached check; the owner's own is the same run.
    assert_eq!(run_of(&h, "chk2").unwrap().status, RunStatus::Passed);
    assert!(h.ctx.store.read(|s| s.find_check("chk2").unwrap().ready));
    let runs = std::fs::read_dir(h.dir.path().join("data/runs"))
        .unwrap()
        .count();
    assert_eq!(runs, 1, "no second run");
}

#[tokio::test]
async fn an_attached_call_returns_at_its_deadline_and_the_run_still_reaches_the_check() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    add_check(&h, &ws, "chk2");
    let first = begin(&h, "chk", &ws, Some("k")).await.unwrap();
    // The call's deadline is shared: 400 ms here, whatever the run does.
    let until = Instant::now() + Duration::from_millis(400);
    let begun = Instant::now();
    let run = iii_cloud::run::start_with(
        &h.ctx,
        "chk2".into(),
        selected_of(&h, &ws),
        Some("k".into()),
        until,
    )
    .await
    .unwrap();
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert_eq!(
        (run.id.as_str(), run.status),
        (first.id.as_str(), RunStatus::Running)
    );
    // Later the owner still publishes the end into the attached check.
    set_workers(&h, ALL);
    assert_eq!(ended(&h, "chk2").await.status, RunStatus::Passed);
}

#[tokio::test]
async fn an_attached_call_returns_when_the_run_says_who_it_waits_on() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, SLOW_START).await;
    let bin = fake_iii(&h, BETA_LOG);
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    add_check(&h, &ws, "chk2");
    begin(&h, "chk", &ws, Some("k")).await.unwrap();
    let run = begin(&h, "chk2", &ws, Some("k")).await.unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.waiting_on.unwrap().containers, vec!["beta"]);
}

#[tokio::test]
async fn another_environment_or_another_key_replaces_instead_of_attaching() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/teams/{TEAM}/environments/33333333-3333-4333-8333-333333333333"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": "33333333-3333-4333-8333-333333333333", "project_id": "p", "name": "staging", "production": false, "branch": "main", "manifest": "worker-compose.yaml", "url": null, "settings": {}}),
        ))
        .mount(&server)
        .await;
    add_check(&h, &ws, "chk2");
    add_check(&h, &ws, "chk3");
    let a = begin(&h, "chk", &ws, Some("k")).await.unwrap();
    // The same commit and key, another environment (two environments share a checkout path).
    let mut other_env = selected_of(&h, &ws);
    other_env.env.id = "33333333-3333-4333-8333-333333333333".into();
    let until = Instant::now() + Duration::from_secs(20);
    let b = iii_cloud::run::start_with(
        &h.ctx,
        "chk2".into(),
        other_env.clone(),
        Some("k".into()),
        until,
    )
    .await
    .unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(
        run_of(&h, "chk").unwrap().error.unwrap().code,
        "RUN_REPLACED"
    );
    // The same environment but not the same key (say, other uncommitted values).
    let c = iii_cloud::run::start_with(&h.ctx, "chk3".into(), other_env, Some("k2".into()), until)
        .await
        .unwrap();
    assert_ne!(b.id, c.id);
    assert_eq!(
        run_of(&h, "chk2").unwrap().error.unwrap().code,
        "RUN_REPLACED"
    );
    // A call with no key never attaches either.
    let d = begin(&h, "chk", &ws, None).await.unwrap();
    assert_ne!(c.id, d.id);
}

#[tokio::test]
async fn the_replacing_run_is_of_the_commit_the_workspace_has_when_it_takes_the_slot() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    // The old run takes a second to stop: a commit made meanwhile must be the new run's.
    let bin = fake_iii(&h, "trap 'sleep 1; exit 0' TERM; sleep 20 & wait;");
    use_iii(&h, &server, &bin).await;
    std::fs::write(
        ws.join(".git/info/exclude"),
        "*.iii-cloud-run.*\n.iii\nconfig\n",
    )
    .unwrap();
    begin(&h, "chk", &ws, Some("k")).await.unwrap();
    add_check(&h, &ws, "chk2");
    // Let compose start and set its trap, or the teardown is instant.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (hh, sel) = (h.ctx.clone(), selected_of(&h, &ws));
    let replacing = tokio::spawn(async move {
        let until = Instant::now() + Duration::from_secs(20);
        iii_cloud::run::start_with(&hh, "chk2".into(), sel, Some("k2".into()), until).await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(ws.join("late.txt"), "late").unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "late"]);
    let late = git(&ws, &["rev-parse", "HEAD"]);
    let run = replacing.await.unwrap().unwrap();
    assert_eq!(run.commit, late);
}

fn alive(pid: u32) -> bool {
    // Portable (Linux and macOS): the pid exists, and is not a zombie (nothing runs).
    // SAFETY: signal 0 only checks that the process exists.
    let exists = unsafe { libc::kill(pid as i32, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    exists
        && std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                let stat = String::from_utf8_lossy(&o.stdout);
                !stat.trim().is_empty() && !stat.trim().starts_with('Z')
            })
}

#[tokio::test]
async fn a_replaced_run_leaves_no_process_of_its_stage_behind() {
    let server = MockServer::start().await;
    let compose = "containers:\n  alpha:\n    worker: package://foo\ncloud:\n  build:\n    command: echo $$ > child.pid; sleep 20 & echo $! > grandchild.pid; wait\n";
    let (h, ws) = with_check(&server, compose).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    begin(&h, "chk", &ws, Some("k")).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let read = |name: &str| {
        std::fs::read_to_string(ws.join(name))
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
    };
    let (child, grandchild) = loop {
        if let (Some(c), Some(g)) = (read("child.pid"), read("grandchild.pid")) {
            break (c, g);
        }
        assert!(Instant::now() < deadline, "the build never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(alive(child) && alive(grandchild));
    add_check(&h, &ws, "chk2");
    // The replacement returns once the old run's teardown is over.
    begin(&h, "chk2", &ws, Some("k2")).await.unwrap();
    assert_eq!(
        run_of(&h, "chk").unwrap().error.unwrap().code,
        "RUN_REPLACED"
    );
    assert!(!alive(child), "the stage's shell is gone");
    assert!(!alive(grandchild), "what it started is gone too");
}

/// Compose takes 3 s to stop once told to.
const SLOW_TEARDOWN: &str = "trap 'sleep 3; exit 0' TERM; sleep 20 & wait;";

#[tokio::test]
async fn a_slow_teardown_does_not_hold_the_call_past_its_deadline() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, SLOW_TEARDOWN);
    use_iii(&h, &server, &bin).await;
    let first = begin(&h, "chk", &ws, Some("k")).await.unwrap();
    add_check(&h, &ws, "chk2");
    // Let compose start and set its trap.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let begun = Instant::now();
    let until = begun + Duration::from_millis(300);
    let err = iii_cloud::run::start_with(
        &h.ctx,
        "chk2".into(),
        selected_of(&h, &ws),
        Some("k2".into()),
        until,
    )
    .await
    .unwrap_err();
    assert!(
        begun.elapsed() < Duration::from_secs(2),
        "not the 3 s of the teardown"
    );
    // The old run is not this call's run: nothing started, and the answer says so.
    assert_eq!(err.code, "RUN_TIMEOUT");
    assert!(err.message.contains("still stopping"), "{}", err.message);
    assert!(run_of(&h, "chk2").is_none());
    let _ = first;
}

#[tokio::test]
async fn a_run_already_told_to_stop_is_not_attached_to() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, TWO_WORKERS).await;
    let bin = fake_iii(&h, SLOW_TEARDOWN);
    use_iii(&h, &server, &bin).await;
    add_check(&h, &ws, "chk2");
    add_check(&h, &ws, "chk3");
    begin(&h, "chk", &ws, Some("k")).await.unwrap();
    // Let compose start and set its trap.
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Another environment's call replaces it and waits for its teardown.
    let mut other = selected_of(&h, &ws);
    other.env.id = "33333333-3333-4333-8333-333333333333".into();
    let hh = h.ctx.clone();
    tokio::spawn(async move {
        let until = Instant::now() + Duration::from_secs(20);
        let _ =
            iii_cloud::run::start_with(&hh, "chk2".into(), other, Some("k2".into()), until).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The first environment asks again with its key: the dying run is not its run.
    let until = Instant::now() + Duration::from_millis(300);
    let _ = iii_cloud::run::start_with(
        &h.ctx,
        "chk3".into(),
        selected_of(&h, &ws),
        Some("k".into()),
        until,
    )
    .await;
    let slot = ws.to_string_lossy().into_owned();
    let runs = h.ctx.runs.lock().unwrap();
    let followers = runs.get(&slot).unwrap().followers.lock().unwrap().clone();
    assert!(
        followers.is_empty(),
        "it attached to the dying run: {followers:?}"
    );
}

#[tokio::test]
async fn a_list_that_cannot_be_read_neither_clears_the_wait_nor_resets_its_clock() {
    let server = MockServer::start().await;
    let (h, _ws) = with_check(&server, SLOW_START).await;
    let bin = fake_iii(&h, BETA_LOG);
    use_iii(&h, &server, &bin).await;
    set_workers(&h, ONLY_ALPHA);
    start(&h.ctx, "chk".into()).await.unwrap();
    let since = waiting(&h, "chk").await.waiting_on.unwrap().since;
    // The engine's list times out for 200 ms, then answers as before.
    set_workers(&h, "not json");
    let mut seen = vec![];
    for step in 0..40 {
        if step == 20 {
            set_workers(&h, ONLY_ALPHA);
        }
        seen.push(run_of(&h, "chk").unwrap());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for run in seen.iter().filter(|r| r.status == RunStatus::Running) {
        let w = run.waiting_on.as_ref().expect("still waiting");
        assert_eq!(w.since, since);
        assert_eq!(run.workers.ready, 1, "the count is kept");
    }
}

#[test]
fn a_container_counts_as_exited_only_when_seen_dead_twice_in_a_row() {
    use iii_cloud::run::confirm_exited;
    let dead = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut before = vec![];
    assert!(
        confirm_exited(&mut before, dead(&["beta"])).is_empty(),
        "one look"
    );
    assert!(
        confirm_exited(&mut before, dead(&[])).is_empty(),
        "it was restarting"
    );
    assert!(confirm_exited(&mut before, dead(&["beta"])).is_empty());
    assert_eq!(confirm_exited(&mut before, dead(&["beta"])), vec!["beta"]);
}

/// Through the workers loop: `iii compose`'s record shows a container dead once, then
/// alive, then dead for good. The run must not fail at the first sight.
#[tokio::test]
async fn the_loop_fails_a_container_only_after_two_looks_in_a_row_find_it_dead() {
    let server = MockServer::start().await;
    let (h, ws) = with_check(&server, SLOW_START).await;
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = gone.id();
    gone.wait().unwrap();
    let state = ws.join(".iii/compose/iii-cloud-run/state.json");
    let rec = |pid: u32| {
        format!("{{\"containers\":{{\"beta\":{{\"pid\":{pid},\"status\":\"ready\"}}}}}}")
    };
    let dir = h.dir.path().join("fake-iii");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("dead.json"), rec(dead_pid)).unwrap();
    // A pid that is alive: this test process's.
    std::fs::write(dir.join("alive.json"), rec(std::process::id())).unwrap();
    let bin = dir.join("iii");
    // Each poll of the engine moves the record on: the 1st call says alive, the rest dead. The record
    // starts alive: `compose` runs beside the first poll, and a start that wrote "dead" after that poll's
    // "alive" made two dead looks out of two calls.
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'iii 0.27.0';;\n  compose) mkdir -p '{st}'; cp '{d}/alive.json' '{s}'; exec sleep 20;;\n  call) n=$(cat '{d}/n' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{d}/n'\n    if [ $n = 1 ]; then cp '{d}/alive.json' '{s}'; else cp '{d}/dead.json' '{s}'; fi\n    cat '{d}/workers.json';;\nesac\n",
            st = state.parent().unwrap().display(),
            s = state.display(),
            d = dir.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_workers(&h, ONLY_ALPHA);
    use_iii(&h, &server, &bin).await;
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
    // Calls 1 and 2 (an alive look, a first dead look) and 3 came before the verdict.
    let calls: u32 = std::fs::read_to_string(dir.join("n"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(calls >= 3, "failed at the first dead look: {calls} polls");
}

/// What the run saves while a stage runs is what `recover` kills by: the real save,
/// not a hand-written file.
#[tokio::test]
async fn recover_kills_the_stage_the_run_itself_recorded() {
    let server = MockServer::start().await;
    let compose = "containers:\n  alpha:\n    worker: package://foo\ncloud:\n  build:\n    command: echo $$ > child.pid; sleep 20 & echo $! > grandchild.pid; wait\n";
    let (h, ws) = with_check(&server, compose).await;
    let bin = fake_iii(&h, "");
    use_iii(&h, &server, &bin).await;
    let run = begin(&h, "chk", &ws, Some("k")).await.unwrap();
    let read = |name: &str| {
        std::fs::read_to_string(ws.join(name))
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let (child, grandchild) = loop {
        if let (Some(c), Some(g)) = (read("child.pid"), read("grandchild.pid")) {
            break (c, g);
        }
        assert!(Instant::now() < deadline, "the build never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            h.dir
                .path()
                .join("data/runs")
                .join(&run.id)
                .join("run.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["stage_group"], child);
    assert!(saved["stage_start"].is_string(), "{saved}");
    iii_cloud::run::recover(&h.ctx).await;
    // SIGKILL takes a moment to land.
    let until = Instant::now() + Duration::from_secs(3);
    while (alive(child) || alive(grandchild)) && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !alive(child) && !alive(grandchild),
        "the recorded stage was left running"
    );
}

/// A graceful stop of the worker drops the run's task: `run.json` then keeps no id of
/// the stage whose group went with it (after a reboot the id could be another process).
#[test]
fn a_graceful_stop_leaves_no_stage_id_in_run_json() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (h, run_json, child) = rt.block_on(async {
        let server = MockServer::start().await;
        let compose = "containers:\n  alpha:\n    worker: package://foo\ncloud:\n  build:\n    command: echo $$ > child.pid; sleep 20 & wait\n";
        let (h, ws) = with_check(&server, compose).await;
        let bin = fake_iii(&h, "");
        use_iii(&h, &server, &bin).await;
        let run = begin(&h, "chk", &ws, Some("k")).await.unwrap();
        let run_json = h.dir.path().join("data/runs").join(&run.id).join("run.json");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let started = std::fs::read_to_string(&run_json).is_ok_and(|t| t.contains("\"stage_group\":") && !t.contains("\"stage_group\":null"));
            if started && ws.join("child.pid").exists() {
                break;
            }
            assert!(Instant::now() < deadline, "the build never started");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let child: u32 = std::fs::read_to_string(ws.join("child.pid")).unwrap().trim().parse().unwrap();
        (h, run_json, child)
    });
    drop(rt);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_json).unwrap()).unwrap();
    assert!(saved["stage_group"].is_null(), "{saved}");
    assert!(!alive(child), "the stage's group went with the task");
    drop(h);
}
