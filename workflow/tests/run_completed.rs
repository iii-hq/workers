//! Engine-level checks of `workflow::on-run-completed`: the worker registers
//! it private with the event as request and response, its handler returns the
//! event, and an emitted event reaches an observer whose filters match it.
//!
//! The engine binary is `III_ENGINE_BIN`, or `iii` on PATH, started with an
//! engine-only config on an ephemeral port. Without either the test skips;
//! `III_E2E_REQUIRE=1` makes a missing or unreachable engine a failure.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout};

use workflow::events::{emit_run_completed, RunCompletedEvent, ON_RUN_COMPLETED_FN};
use workflow::functions::{self, Deps};
use workflow::types::{RunStatus, WorkflowRunRecord};

const TARGET_FN: &str = "test::workflow-run-failed";

struct Engine {
    url: String,
    child: Child,
    dir: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn e2e_required() -> bool {
    std::env::var("III_E2E_REQUIRE").is_ok_and(|v| v == "1")
}

fn engine_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("III_ENGINE_BIN") {
        return Some(path.into());
    }
    let on_path = Command::new("iii")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    client
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(2000),
        })
        .await
}

async fn spawn_engine() -> Result<Engine, String> {
    let bin = engine_bin().ok_or("no engine binary (set III_ENGINE_BIN or put `iii` on PATH)")?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("workflow-it-{}-{port}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).map_err(|e| format!("create {dir:?}: {e}"))?;
    let config = format!(
        "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: \"{}\"\n",
        dir.join("config").display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .map_err(|e| format!("write {config_path:?}: {e}"))?;

    let child = Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {bin:?}: {e}"))?;
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child,
        dir,
    };

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    while call(&probe, "engine::workers::list", json!({})).await.is_err() {
        if Instant::now() > deadline {
            probe.shutdown_async().await;
            return Err(format!("engine {bin:?} did not come up on {}", engine.url));
        }
        sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;
    Ok(engine)
}

fn record(run_id: &str, status: RunStatus) -> WorkflowRunRecord {
    WorkflowRunRecord {
        run_id: run_id.to_string(),
        step: 3,
        status,
        abort: false,
        def_ref: run_id.to_string(),
        input: json!({}),
        nodes: BTreeMap::new(),
        fanout_src: BTreeMap::new(),
        result: (status == RunStatus::Completed).then(|| json!({ "answer": 42 })),
        result_error: (status == RunStatus::Failed).then(|| "node 'a': boom".to_string()),
        notify: None,
        reply_to: None,
        caller_session_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Worker names are unique per namespace, so each client of this process
/// names itself.
fn named(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: name.to_string(),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn run_completed_is_private_returns_the_event_and_reaches_matching_observers() {
    let engine = match spawn_engine().await {
        Ok(engine) => engine,
        Err(reason) if !e2e_required() => {
            eprintln!("skipping: {reason}");
            return;
        }
        Err(reason) => panic!("{reason}"),
    };

    // The workflow worker, registered the way main.rs does it.
    let host = Arc::new(register_worker(&engine.url, named("workflow")));
    let deps = Deps {
        iii: host.clone(),
        cfg: Arc::new(RwLock::new(Arc::new(
            workflow::config::WorkerConfig::default(),
        ))),
        locks: workflow::locks::WorkflowLocks::default(),
        events: host.clone(),
    };
    functions::register_all(&host, &deps);

    // Registered private, with the event as request and response.
    let observer = register_worker(&engine.url, named("workflow-test-observer"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let info = loop {
        match call(
            &observer,
            "engine::functions::info",
            json!({ "function_id": ON_RUN_COMPLETED_FN }),
        )
        .await
        {
            Ok(info) => break info,
            Err(e) => {
                assert!(Instant::now() < deadline, "{ON_RUN_COMPLETED_FN} never registered: {e}");
                sleep(Duration::from_millis(100)).await;
            }
        }
    };
    assert_eq!(info["private"], json!(true), "{info}");
    for schema in ["request_schema", "response_schema"] {
        assert!(
            info[schema]["properties"]["run_id"].is_object()
                && info[schema]["properties"]["status"].is_object(),
            "{schema} is the RunCompletedEvent: {info}"
        );
    }
    assert!(info.get("args_format").is_none(), "takes no args: {info}");

    // Its handler returns the event it is called with.
    let event = json!({
        "run_id": "r-direct",
        "status": "completed",
        "result": { "answer": 42 },
        "result_error": null,
    });
    let returned = call(&host, ON_RUN_COMPLETED_FN, event.clone())
        .await
        .expect("the host calls its own function");
    assert_eq!(returned, event);

    // Unobserved: the emitter's gate is closed.
    assert!(!host.is_observed(ON_RUN_COMPLETED_FN));

    // An observer of failed runs only.
    let (tx, mut rx) = mpsc::unbounded_channel::<RunCompletedEvent>();
    observer.register_function(
        TARGET_FN,
        RegisterFunction::new(move |event: RunCompletedEvent| {
            let _ = tx.send(event);
            Ok::<_, Error>(json!({}))
        }),
    );
    let handle = observer.observe(
        Source::function(ON_RUN_COMPLETED_FN)
            .filters(json!({ "input.status": { "$eq": "failed" } }))
            .deliver(Deliver::Output),
        TARGET_FN,
    );
    timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer became active in time")
        .expect("observer active");
    wait_until("the host to see the observer", || {
        host.is_observed(ON_RUN_COMPLETED_FN)
    })
    .await;

    emit_run_completed(host.as_ref(), &record("r-ok", RunStatus::Completed)).await;
    emit_run_completed(host.as_ref(), &record("r-fail", RunStatus::Failed)).await;

    let received = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the failed run reached the observer")
        .expect("channel open");
    assert_eq!(
        received,
        RunCompletedEvent {
            run_id: "r-fail".into(),
            status: RunStatus::Failed,
            result: None,
            result_error: Some("node 'a': boom".into()),
        }
    );
    // The completed run does not match the observer's filters.
    assert!(
        timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "only the failed run is delivered"
    );

    handle.dispose().await.expect("dispose the observer");
    observer.shutdown_async().await;
    host.shutdown_async().await;
}
