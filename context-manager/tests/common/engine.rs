//! Connect-or-skip wrapper around the iii SDK.
//!
//! One engine connection per test binary process via OnceCell. On
//! success the production surface (the four `context::*` functions
//! with production adapters) is registered in-process so @engine
//! scenarios drive the exact code the binary would run.
//!
//! The engine is the one `III_ENGINE_WS_URL` names, which `bdd.rs` points
//! at a private engine from [`spawn_private`] unless the caller set it.
//! A developer's engine on the default port is never joined.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::protocol::CallRequest;
use iii_sdk::{IIIClient, InitOptions, register_worker};
use serde_json::json;
use tokio::sync::OnceCell;

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

/// Kills and reaps the engine on drop, so a panic cannot orphan it.
pub struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `iii` on a free loopback port with `dir` as its `HOME` and working
/// directory, and return it with its ws URL. `None` when `iii` is not on
/// PATH or never listens, so the @engine scenarios soft-skip.
pub fn spawn_private(dir: &Path) -> Option<(Reaped, String)> {
    let iii_bin = which::which("iii").ok()?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("free port")
        .port();
    let config = dir.join("engine.yaml");
    std::fs::write(
        &config,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
        ),
    )
    .expect("write engine config");
    let engine = Command::new(iii_bin)
        .args(["--no-update-check", "--config"])
        .arg(&config)
        .current_dir(dir)
        .env("HOME", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Reaped)
        .inspect_err(|error| eprintln!("[skip] spawning iii failed: {error}"))
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() >= deadline {
            eprintln!("[skip] the private iii engine never listened on {port}");
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Some((engine, format!("ws://127.0.0.1:{port}")))
}

async fn try_connect_raw() -> Option<Arc<IIIClient>> {
    let Ok(url) = std::env::var("III_ENGINE_WS_URL") else {
        eprintln!(
            "[skip] no iii engine; put `iii` on PATH or set III_ENGINE_WS_URL \
             to enable engine-bound BDD scenarios"
        );
        return None;
    };
    let iii = Arc::new(register_worker(&url, InitOptions::default()));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        match probe {
            Ok(_) => return Some(iii),
            Err(Error::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    eprintln!(
        "[skip] iii engine not reachable at {url}; \
         set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
    );
    iii.shutdown_async().await;
    None
}

/// Get-or-init the shared engine handle, registering the production
/// context-manager surface in-process on first success.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            super::workers::register_all(&iii).await;
            Some(iii)
        })
        .await
        .clone()
}
