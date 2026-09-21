//! Minimal engine lifecycle for the standalone JEV bus tests.
use std::fs::File;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tempfile::TempDir;

pub struct Engine {
    child: Child,
    _root: TempDir,
    pub url: String,
}

impl Engine {
    pub async fn start(case: &str) -> Self {
        let binary = std::env::var("III_ENGINE_BIN").expect("set III_ENGINE_BIN");
        assert!(
            PathBuf::from(&binary).is_absolute(),
            "III_ENGINE_BIN must be absolute"
        );
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("engine.yaml");
        std::fs::write(
            &config,
            format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
        )
        .unwrap();
        // Keep logs outside the scratch directory so panic cleanup preserves them.
        let reports = std::env::var_os("JEV_E2E_REPORT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("jev-e2e-{}", std::process::id()))
            });
        std::fs::create_dir_all(&reports).unwrap();
        let log_path = reports.join(format!("{case}.engine.log"));
        let log = File::create(&log_path).unwrap();
        eprintln!("Engine log: {}", log_path.display());
        let child = Command::new(binary)
            .args(["--no-update-check", "--config"])
            .arg(config)
            .current_dir(root.path())
            .env_remove("TYPESAFE_API_KEY")
            .env("RUST_LOG", "info")
            .env("OTEL_ENABLED", "false")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let engine = Self {
            child,
            _root: root,
            url: format!("ws://127.0.0.1:{port}"),
        };
        let probe = connect(&engine.url, "jev-engine-probe").await;
        wait_for(&probe, "engine::functions::list").await;
        probe.shutdown_async().await;
        engine
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub async fn connect(url: &str, name: &str) -> IIIClient {
    let iii = register_worker(
        url,
        InitOptions {
            metadata: Some(iii_sdk::runtime::WorkerMetadata {
                name: name.into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("engine connection; inspect the preserved engine log");
    iii
}

pub async fn invoke(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::errors::Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(5000),
    })
    .await
}

pub async fn wait_for(iii: &IIIClient, function_id: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(value) = invoke(
                iii,
                "engine::functions::list",
                json!({"include_internal": true}),
            )
            .await
            {
                if value.to_string().contains(&format!("\"{function_id}\"")) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("registered function became discoverable");
}
