//! Engine-level check of the boot observers: the approval observers exist
//! whether or not approval-gate is connected, stay `pending` while it is
//! absent and become `active` when it registers its functions.
//!
//! The engine binary is `III_ENGINE_BIN`, or `iii` on PATH, started with an
//! engine-only config on an ephemeral port. Without either the test skips;
//! `III_E2E_REQUIRE=1` makes a missing or unreachable engine a failure.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, CallRequest, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::time::sleep;

use slack::deps::Deps;
use slack::functions::bindings;
use slack::WorkerConfig;

const APPROVAL_FNS: [&str; 2] = [
    bindings::APPROVAL_PENDING_CREATED_FN,
    bindings::APPROVAL_PENDING_RESOLVED_FN,
];

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
    let dir = std::env::temp_dir().join(format!("slack-it-{}-{port}", std::process::id()));
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

/// `function_id -> state` of slack's observers of the approval functions.
async fn approval_observer_states(client: &IIIClient) -> Vec<(String, String)> {
    let Ok(list) = call(client, "engine::observers::list", json!({})).await else {
        return Vec::new();
    };
    let mut states: Vec<(String, String)> = list["observers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["worker"]["worker_name"] == "slack")
        .filter_map(|o| {
            let function_id = o["function_id"].as_str()?;
            let state = o["state"].as_str()?;
            APPROVAL_FNS
                .contains(&function_id)
                .then(|| (function_id.to_string(), state.to_string()))
        })
        .collect();
    states.sort();
    states
}

async fn wait_for_states(client: &IIIClient, state: &str) -> Vec<(String, String)> {
    let want: Vec<(String, String)> = APPROVAL_FNS
        .iter()
        .map(|f| (f.to_string(), state.to_string()))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let states = approval_observer_states(client).await;
        if states == want || Instant::now() > deadline {
            return states;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn approval_observers_exist_without_the_gate_and_activate_when_it_registers() {
    let engine = match spawn_engine().await {
        Ok(engine) => engine,
        Err(reason) if !e2e_required() => {
            eprintln!("skipping: {reason}");
            return;
        }
        Err(reason) => panic!("{reason}"),
    };

    // Slack's targets and boot observers, the way main.rs creates them.
    let worker = Arc::new(register_worker(&engine.url, named("slack")));
    let cell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
    let deps = Arc::new(Deps::new(worker.clone(), cell));
    bindings::register(&worker, &deps);
    bindings::observe_events(&worker);

    let inspector = register_worker(&engine.url, named("slack-test-inspector"));
    let states = wait_for_states(&inspector, "pending").await;
    assert_eq!(
        states,
        vec![
            (APPROVAL_FNS[0].to_string(), "pending".to_string()),
            (APPROVAL_FNS[1].to_string(), "pending".to_string()),
        ],
        "both approval observers exist and wait for approval-gate"
    );

    // approval-gate connects after slack and registers its event functions.
    let gate = register_worker(&engine.url, named("approval-gate"));
    for id in APPROVAL_FNS {
        gate.register_function(
            id,
            RegisterFunction::new(|event: Value| Ok::<_, Error>(event)).private(),
        );
    }
    let states = wait_for_states(&inspector, "active").await;
    assert_eq!(
        states,
        vec![
            (APPROVAL_FNS[0].to_string(), "active".to_string()),
            (APPROVAL_FNS[1].to_string(), "active".to_string()),
        ],
        "the observers become active once approval-gate registers"
    );

    gate.shutdown_async().await;
    inspector.shutdown_async().await;
    worker.shutdown_async().await;
}
