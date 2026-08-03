//! Engine-backed test bootstrap. Self-skips when no `iii` binary is on PATH
//! and `III_ENGINE_BIN` is unset, so CI and casual local runs stay green.
//!
//! The worker's functions are registered in process against a real engine
//! rather than by spawning the binary. That is the point of these tests: they
//! exercise the actual wire path, where serde silently drops a field a unit
//! test would never notice.

#![allow(dead_code)]

use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use pdf::config::WorkerConfig;
use pdf::configuration::ConfigCell;

pub struct Engine {
    pub url: String,
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn engine_bin() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("III_ENGINE_BIN") {
        return Some(p.into());
    }
    let on_path = std::process::Command::new("iii")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// `true` once the configuration worker answers at all. A not-found for a
/// nonexistent id still proves it is serving, which is what we need to know.
async fn configuration_serving(probe: &IIIClient) -> bool {
    match probe
        .trigger(TriggerRequest {
            function_id: "configuration::get".into(),
            payload: json!({ "id": "__readiness_probe__" }),
            action: None,
            timeout_ms: Some(1000),
        })
        .await
    {
        Ok(_) => true,
        Err(e) => e.to_string().to_ascii_uppercase().contains("NOT_FOUND"),
    }
}

async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("pdf-it-{}-{port}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    // The configuration worker's fs adapter writes one YAML file per id here;
    // create it up front so register and get never race a missing path.
    std::fs::create_dir_all(dir.join("configuration")).ok()?;

    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: "{dir}/configuration"
      ttl_seconds: 0
"#,
        port = port,
        dir = dir.display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .ok()?;

    let child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // The engine core must be up AND the configuration worker serving:
        // this worker treats configuration as a required boot dependency, so
        // starting before it is ready would fail spuriously. Any response to
        // the probe id counts, including a not-found.
        let core_ready = probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1000),
            })
            .await
            .is_ok();

        if core_ready && configuration_serving(&probe).await {
            break;
        }
        if Instant::now() > deadline {
            probe.shutdown_async().await;
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;

    Some(Engine { url, child, dir })
}

pub struct Stack {
    pub iii: Arc<IIIClient>,
    _engine: Engine,
}

impl Stack {
    /// Invoke one of this worker's functions over the bus, the way a caller
    /// would.
    pub async fn call(&self, function_id: &str, payload: Value) -> Result<Value, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(30_000),
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// Boot an engine and register this worker against it, or `None` when no
/// engine binary is available.
pub async fn boot() -> Option<Stack> {
    let engine = spawn_engine().await?;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
    pdf::functions::register_all(&iii, &cell);

    // Let the registrations land before the first call.
    tokio::time::sleep(Duration::from_millis(500)).await;

    Some(Stack {
        iii,
        _engine: engine,
    })
}

/// Run `f` against a freshly booted stack; skip when no engine is available.
pub async fn with_stack<F, Fut>(f: F)
where
    F: FnOnce(Stack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(stack) = boot().await else {
        eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
        return;
    };
    f(stack).await;
}

/// Absolute path to a committed fixture.
pub fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}
