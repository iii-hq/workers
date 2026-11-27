//! What a hung Docker does to the runs: the slot is freed, the container stays on
//! record, the prune keeps its folder, and a restart's workspace-keyed clean-up is
//! over before the functions are registered.
//!
//! One test, in a process of its own: it puts a fake `docker` first on `PATH` and
//! shortens the clean-up timeout once, before any thread reads the environment.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iii_cloud::run::{clean_stale, recover_runs, start};
use iii_cloud::types::*;
use serde_json::json;
use support::repo::*;
use wiremock::MockServer;

const COMPOSE: &str = "containers:\n  db:\n    worker: package://database\n";

/// Fake `docker` (a `hang` file makes `rm` take 5 s) and fake `iii`; their calls are logged.
fn fakes(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let write = |name: &str, text: String| {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let d = dir.display();
    write(
        "docker",
        format!(
            "#!/bin/sh\necho \"$@\" >> '{d}/docker-calls'\nif [ \"$1\" = rm ] && [ -f '{d}/hang' ]; then exec sleep 5; fi\nexit 0\n"
        ),
    );
    write(
        "iii",
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'iii 0.27.0';;\n  compose) exec sleep 20;;\n  kill) echo \"$@\" >> '{d}/iii-calls';;\n  call) echo '{{\"workers\":[{{\"name\":\"db\",\"status\":\"connected\"}}]}}';;\nesac\n"
        ),
    );
}

async fn workspace(server: &MockServer) -> (support::Harness, PathBuf) {
    let h = connected(server, "worker-compose.yaml").await;
    let ws = h.dir.path().join("app");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("worker-compose.yaml"), COMPOSE).unwrap();
    git(&ws, &["init", "-b", "main"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "app"]);
    // The run's own files are excluded in a real workspace.
    std::fs::write(
        ws.join(".git/info/exclude"),
        "*.iii-cloud-run.*\n.iii\nconfig\n",
    )
    .unwrap();
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

/// One whole run (its teardown too); returns its id and how long the slot stayed taken after the run ended.
async fn one_run(h: &support::Harness) -> (String, Duration) {
    let began = Instant::now();
    let run = start(&h.ctx, "chk".into()).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let r = h
            .ctx
            .store
            .read(|s| s.find_check("chk").and_then(|c| c.run.clone()))
            .unwrap();
        if r.id == run.id && r.finished_at.is_some() {
            assert_eq!(
                r.status,
                RunStatus::Passed,
                "{:?} {:?}",
                r.error,
                r.log_tail
            );
            break;
        }
        assert!(Instant::now() < deadline, "the run never ended");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ended = Instant::now();
    while !h.ctx.runs.lock().unwrap().is_empty() {
        assert!(
            ended.elapsed() < Duration::from_secs(20),
            "the slot is never freed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (run.id, began.elapsed())
}

fn record(data: &Path, id: &str) -> serde_json::Value {
    let text = std::fs::read(data.join("runs").join(id).join("run.json")).unwrap();
    serde_json::from_slice(&text).unwrap()
}

#[test]
fn a_hung_docker_neither_holds_the_slot_nor_makes_a_container_forgotten() {
    let fake_dir = tempfile::tempdir().unwrap();
    fakes(fake_dir.path());
    // SAFETY: nothing else runs yet: this is the file's only test, and no runtime exists.
    unsafe {
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                fake_dir.path().display(),
                std::env::var("PATH").unwrap()
            ),
        );
        std::env::set_var("III_CLOUD_CLEANUP_TIMEOUT_MS", "1000");
    }
    let hang = fake_dir.path().join("hang");
    let docker_calls =
        || std::fs::read_to_string(fake_dir.path().join("docker-calls")).unwrap_or_default();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        let (h, _ws) = workspace(&server).await;
        *h.ctx.cfg.write().await = std::sync::Arc::new(iii_cloud::config::Config {
            cloud_url: server.uri(),
            iii_bin: fake_dir.path().join("iii").to_string_lossy().into(),
            data_dir: h.dir.path().join("data").to_string_lossy().into(),
            ..iii_cloud::config::Config::default()
        });
        let data = h.dir.path().join("data");

        // 1. `docker rm` hangs (5 s; the teardown gives up after 1 s): the slot is free
        //    well before the 5 s, so the next call gets it.
        std::fs::write(&hang, "").unwrap();
        let (a, took) = one_run(&h).await;
        eprintln!("whole run and slot: {took:?}");
        assert!(took < Duration::from_secs(4), "the run held the slot {took:?}: a hung docker (5 s) blocked it");
        assert!(record(&data, &a)["copy"].is_null(), "the record names a copy the teardown deleted");
        let name_a = record(&data, &a)["containers"][0].as_str().unwrap().to_string();
        assert!(name_a.starts_with("iii-cloud-run-pg-"), "the left container is not on record");
        let (b, _) = one_run(&h).await;

        // 2. The prune keeps the folders whose containers could not be removed.
        for i in 0..22 {
            std::fs::create_dir_all(data.join("runs").join(format!("dummy-{i:02}"))).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        one_run(&h).await;
        for id in [&a, &b] {
            assert!(data.join("runs").join(id).join("run.json").exists(), "a folder naming a container was pruned");
        }

        // 3. Docker answers again: the next prune removes the containers, then the folders.
        std::fs::remove_file(&hang).unwrap();
        // A run that is going has its Postgres on record from the start: the prune never touches its folder.
        let live = data.join("runs/live-1");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(
            live.join("run.json"),
            json!({"containers": ["iii-cloud-run-pg-live-0"], "copy": null, "iii_bin": "iii"}).to_string(),
        )
        .unwrap();
        std::fs::File::open(&live)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
        iii_cloud::run::set_run_live(&live, true);
        one_run(&h).await;
        // The prune runs after the slot is given back: wait for it.
        let gone = |id: &String| !data.join("runs").join(id).exists();
        for _ in 0..500 {
            if gone(&a) && gone(&b) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        for id in [&a, &b] {
            assert!(gone(id), "the folder stayed after its container went");
        }
        assert!(docker_calls().contains(&format!("rm -f {name_a}")));
        assert!(live.join("run.json").exists(), "the prune took a live run's folder");
        assert!(!docker_calls().contains("iii-cloud-run-pg-live-0"), "the prune removed a live run's database");
        iii_cloud::run::set_run_live(&live, false);

        // 4. A restart: what is keyed by the workspace is over when `recover_runs` returns
        //    (before the functions are registered); only the containers wait for `clean_stale`.
        std::fs::write(&hang, "").unwrap();
        let app = h.dir.path().join("app");
        let copy = app.join("worker-compose.iii-cloud-run.yaml");
        std::fs::write(&copy, "containers: {}\n").unwrap();
        std::fs::create_dir_all(app.join(".iii/compose/iii-cloud-run")).unwrap();
        let stale_dir = data.join("runs/stale-1");
        std::fs::create_dir_all(&stale_dir).unwrap();
        std::fs::write(
            stale_dir.join("run.json"),
            json!({"containers": ["iii-cloud-run-pg-stale-0"], "copy": copy, "iii_bin": fake_dir.path().join("iii")}).to_string(),
        )
        .unwrap();
        let began = Instant::now();
        let stale = recover_runs(&h.ctx).await;
        assert!(began.elapsed() < Duration::from_millis(900), "recover_runs waited for docker");
        let iii_calls = std::fs::read_to_string(fake_dir.path().join("iii-calls")).unwrap_or_default();
        assert!(iii_calls.contains("kill --compose"), "`iii kill` is not done when recover_runs returns");
        assert!(!copy.exists() && !app.join(".iii/compose/iii-cloud-run").exists());
        let kept = record(&data, "stale-1");
        assert_eq!(kept["containers"][0], "iii-cloud-run-pg-stale-0");
        assert!(kept["copy"].is_null(), "the record still names a copy that is gone");
        // The containers go afterwards, in the background.
        std::fs::remove_file(&hang).unwrap();
        clean_stale(stale).await;
        assert!(!stale_dir.join("run.json").exists());
        assert!(docker_calls().contains("rm -f iii-cloud-run-pg-stale-0"));
    });
}
