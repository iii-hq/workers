use std::fs::File;
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use iii_sdk::errors::Error as IiiError;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tempfile::TempDir;

pub(crate) struct Engine {
    pub(crate) url: String,
    child: Child,
    _directory: TempDir,
}

impl Engine {
    pub(crate) async fn start() -> anyhow::Result<Self> {
        let binary = std::env::var_os("III_ENGINE_BIN")
            .map(PathBuf::from)
            .context("III_ENGINE_BIN is required for provider contracts")?;
        if !binary.is_file() {
            bail!(
                "III_ENGINE_BIN does not point to a file: {}",
                binary.display()
            );
        }
        let port = StdTcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let directory = tempfile::tempdir()?;
        let config_path = directory.path().join("config.yaml");
        let config = format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: iii-pubsub
    config:
      adapter:
        name: local
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {directory}/configuration
      ttl_seconds: 0
  - name: iii-state
    config:
      adapter:
        name: kv
        config:
          file_path: {directory}/state.db
          store_method: file_based
"#,
            directory = directory.path().display()
        );
        std::fs::write(&config_path, config)?;
        let stdout = File::create(directory.path().join("engine.stdout.log"))?;
        let stderr = File::create(directory.path().join("engine.stderr.log"))?;
        let child = Command::new(&binary)
            .arg("--no-update-check")
            .arg("--config")
            .arg(&config_path)
            .current_dir(directory.path())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("spawn iii engine {}", binary.display()))?;
        let url = format!("ws://127.0.0.1:{port}");
        wait_for_engine(&url).await?;
        Ok(Self {
            url,
            child,
            _directory: directory,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn test_init_options() -> InitOptions {
    static NEXT_WORKER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut metadata = iii_sdk::runtime::WorkerMetadata::default();
    let worker_id = NEXT_WORKER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    metadata.name = format!("{}-test-{worker_id}", metadata.name);
    InitOptions {
        metadata: Some(metadata),
        ..InitOptions::default()
    }
}

async fn wait_for_engine(url: &str) -> anyhow::Result<()> {
    let probe = register_worker(url, test_init_options());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if call(&probe, "engine::workers::list", json!({}))
            .await
            .is_ok()
        {
            probe.shutdown();
            return Ok(());
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            bail!("iii engine did not become ready in 20 seconds");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, IiiError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(30_000),
    })
    .await
}
