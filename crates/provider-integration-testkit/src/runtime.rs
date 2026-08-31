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
    state_child: Option<Child>,
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
        let state_binary = std::env::var_os("III_STATE_BIN")
            .map(PathBuf::from)
            .context("III_STATE_BIN is required for provider contracts")?;
        if !state_binary.is_file() {
            bail!(
                "III_STATE_BIN does not point to a file: {}",
                state_binary.display()
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
"#
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
        let mut engine = Self {
            url,
            child,
            state_child: None,
            _directory: directory,
        };
        wait_for_engine(&engine.url).await?;
        let state_config_path = engine._directory.path().join("state-config.yaml");
        std::fs::write(
            &state_config_path,
            "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
        )?;
        let state_stdout = File::create(engine._directory.path().join("state.stdout.log"))?;
        let state_stderr = File::create(engine._directory.path().join("state.stderr.log"))?;
        let state_child = Command::new(&state_binary)
            .arg("--url")
            .arg(&engine.url)
            .arg("--config")
            .arg(&state_config_path)
            .current_dir(engine._directory.path())
            .stdout(Stdio::from(state_stdout))
            .stderr(Stdio::from(state_stderr))
            .spawn()
            .with_context(|| format!("spawn state worker {}", state_binary.display()))?;
        engine.state_child = Some(state_child);
        wait_for_state(&engine.url).await?;
        Ok(engine)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(state_child) = self.state_child.as_mut() {
            let _ = state_child.kill();
            let _ = state_child.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_for_state(url: &str) -> anyhow::Result<()> {
    let probe = register_worker(url, test_init_options());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            probe.shutdown();
            bail!("state worker did not become ready in 20 seconds");
        };
        match tokio::time::timeout(
            remaining,
            call(
                &probe,
                "state::get",
                json!({ "scope": "provider-contract", "key": "ready" }),
            ),
        )
        .await
        {
            Ok(Ok(_)) => {
                probe.shutdown();
                return Ok(());
            }
            Ok(Err(_)) => {}
            Err(_) => {
                probe.shutdown();
                bail!("state worker did not become ready in 20 seconds");
            }
        }
        tokio::time::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        )
        .await;
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
