//! An isolated engine process for tests that exercise the real bus.

use std::io::Read as _;
use std::time::{Duration, Instant};

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{CallRequest, InitOptions, register_worker};
use serde_json::json;

pub struct Engine {
    pub url: String,
    child: std::process::Child,
    _dir: tempfile::TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn worker_options(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: name.to_string(),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

pub async fn isolated_engine() -> Option<Engine> {
    isolated_engine_with("").await
}

/// An isolated engine whose config also carries `extra`, top-level YAML
/// such as an `observers:` section.
pub async fn isolated_engine_with(extra: &str) -> Option<Engine> {
    let binary = std::env::var_os("III_ENGINE_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::process::Command::new("iii")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok()
                .filter(|status| status.success())
                .map(|_| std::path::PathBuf::from("iii"))
        })?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let dir = tempfile::tempdir().ok()?;
    let config = dir.path().join("config.yaml");
    std::fs::write(
        &config,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n{extra}"),
    )
    .ok()?;
    let mut child = std::process::Command::new(binary)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config)
        .current_dir(dir.path())
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, worker_options("probe"));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if probe
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            probe.shutdown_async().await;
            return Some(Engine {
                url,
                child,
                _dir: dir,
            });
        }
        if Instant::now() >= deadline {
            probe.shutdown_async().await;
            let _ = child.kill();
            let _ = child.wait();
            if std::env::var_os("III_ENGINE_BIN").is_some() {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("iii engine did not become ready: {stderr}");
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
