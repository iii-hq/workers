//! An isolated real engine (`III_ENGINE_BIN`) on an ephemeral port, for the
//! tests that need iii's own delivery. Tests skip when it is not set.
#![allow(dead_code)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{register_worker, CallRequest, IIIClient, InitOptions};
use serde_json::{json, Value};

pub struct RealEngine {
    child: Child,
    pub url: String,
    _root: tempfile::TempDir,
}

impl RealEngine {
    /// Starts `III_ENGINE_BIN` with only its worker manager listening, on
    /// 127.0.0.1. `None` when the variable is not set.
    pub async fn start() -> Option<Self> {
        let binary = PathBuf::from(std::env::var_os("III_ENGINE_BIN")?);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("engine.yaml");
        std::fs::write(
            &config,
            format!(
                "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
            ),
        )
        .unwrap();
        let child = Command::new(binary)
            .args(["--no-update-check", "--config"])
            .arg(&config)
            .current_dir(root.path())
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let engine = Self {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            _root: root,
        };
        let probe = engine.client("engine-probe").await;
        tokio::time::timeout(Duration::from_secs(30), async {
            while call(&probe, "engine::functions::list", json!({}))
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the engine answers");
        probe.shutdown_async().await;
        Some(engine)
    }

    /// A worker connection named `name`, once it is connected.
    pub async fn client(&self, name: &str) -> Arc<IIIClient> {
        let iii = register_worker(
            &self.url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    name: name.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            while iii.get_connection_state() != IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("connected to the engine");
        Arc::new(iii)
    }

    /// Waits until the engine lists `function_id`, and returns its row.
    pub async fn function(&self, iii: &IIIClient, function_id: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(info) = call(
                    iii,
                    "engine::functions::info",
                    json!({ "function_id": function_id }),
                )
                .await
                {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("the function is registered")
    }
}

impl Drop for RealEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::Error> {
    iii.call(CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}
