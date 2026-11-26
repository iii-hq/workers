//! Crash recovery: a run left `running` is failed, and what it left behind is removed.

mod support;

use std::sync::Arc;

use iii_cloud::run::recover;
use iii_cloud::types::*;
use support::{Fake, harness};

#[tokio::test]
async fn a_restart_fails_the_running_run_and_cleans_up_its_files() {
    let h = harness("http://cloud.test", Arc::new(Fake::default()));
    let cfg = h.ctx.config().await;
    // A run copy and its lock, as a crash leaves them, and the record of what to remove.
    let app = h.dir.path().join("app");
    std::fs::create_dir_all(app.join(".iii/compose/iii-cloud-run")).unwrap();
    let copy = app.join("worker-compose.iii-cloud-run.yaml");
    std::fs::write(&copy, "containers: {}\n").unwrap();
    std::fs::write(copy.with_extension("lock"), "").unwrap();
    let run_dir = cfg.data_path().join("runs/r1");
    std::fs::create_dir_all(&run_dir).unwrap();
    // `true` stands for `iii`: `true kill --compose <copy>` succeeds and does nothing.
    std::fs::write(
        run_dir.join("run.json"),
        serde_json::json!({"containers": ["iii-cloud-run-pg-nonexistent-0"], "copy": copy, "iii_bin": "true"}).to_string(),
    )
    .unwrap();
    let running = Run {
        id: "r1".into(),
        check_id: "c1".into(),
        commit: "abc".into(),
        engine: "0.27.0".into(),
        status: RunStatus::Running,
        started_at: "t".into(),
        finished_at: None,
        stages: StageName::ALL
            .iter()
            .map(|n| Stage {
                name: *n,
                status: if *n == StageName::Workers {
                    StageStatus::Running
                } else {
                    StageStatus::Pending
                },
                detail: None,
                duration_ms: None,
            })
            .collect(),
        workers: WorkerCount { ready: 0, total: 2 },
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
    };
    let (part, _) =
        iii_cloud::check::plan_part(&serde_json::json!({"can_deploy": true, "plan": {}}));
    h.ctx.store.update(|s| {
        s.put_check(Check {
            id: "c1".into(),
            environment_id: "e".into(),
            commit: "abc".into(),
            base: String::new(),
            checked_at: "t".into(),
            duration_ms: 0,
            plan: part,
            here: HerePart {
                problems: 0,
                iii_version: None,
                accepts_cloud_block: true,
            },
            problems: vec![],
            run: Some(running),
            ready: false,
            report: String::new(),
            manifest: None,
            handoff_id: None,
            commits: vec![],
        })
    });
    recover(&h.ctx).await;
    let run = h
        .ctx
        .store
        .read(|s| s.check.as_ref().unwrap().run.clone().unwrap());
    assert_eq!(run.status, RunStatus::Failed);
    let error = run.error.unwrap();
    assert_eq!(
        (error.stage, error.code.as_str()),
        (StageName::Workers, "WORKER_RESTARTED")
    );
    assert!(!copy.exists() && !copy.with_extension("lock").exists());
    assert!(!app.join(".iii/compose/iii-cloud-run").exists());
    assert!(!run_dir.join("run.json").exists());
    assert!(h.fake.kinds().contains(&"check:true".to_string()));
    // Whoever waits for the run hears that it ended.
    let events = h.fake.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e.kind == EventKind::Run
            && e.id == "r1"
            && e.terminal
            && e.record["error"]["code"] == "WORKER_RESTARTED"),
        "{events:?}"
    );
}

/// A stage command runs in its own process group, so the worker's death does
/// not reach it: `recover` kills the group `run.json` names.
#[tokio::test]
async fn a_restart_kills_the_process_group_of_the_stage_that_was_running() {
    use std::os::unix::process::CommandExt;
    let h = harness("http://cloud.test", Arc::new(Fake::default()));
    let cfg = h.ctx.config().await;
    let mut stage = std::process::Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .unwrap();
    let run_dir = cfg.data_path().join("runs/r2");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("run.json"),
        serde_json::json!({"containers": [], "copy": null, "iii_bin": "true", "stage_group": stage.id()}).to_string(),
    )
    .unwrap();
    recover(&h.ctx).await;
    // Killed by SIGKILL: `wait` returns at once, with no exit code.
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || stage.wait().unwrap()),
    )
    .await
    .expect("the stage was left running")
    .unwrap();
    assert!(status.code().is_none());
    assert!(!run_dir.join("run.json").exists());
}
