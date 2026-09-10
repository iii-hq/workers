//! Shared engine + state bootstrap for the engine-backed integration suites
//! (llm-router and every provider). Included by path — no crate, no Cargo
//! plumbing — from each suite:
//!
//! ```ignore
//! #[path = "../../llm-router/tests/support/engine_fixture.rs"]
//! mod engine_fixture;
//! use engine_fixture::*;
//! ```
//!
//! What it spawns, and why this shape: a *bare* engine (`iii-worker-manager`
//! only, on a free port) plus the standalone `state` worker connected to it.
//! Engines since 0.23 refuse `iii-state` / `iii-pubsub` inline in `config.yaml`
//! (`UNSUPPORTED_CONFIG_WORKERS`: they are Compose containers now), which is
//! how eleven copies of the old bootstrap failed every test at the 15 s
//! readiness assert for months while CI, with no `iii` on PATH, skipped them.
//! The router and providers need `state` and nothing else; pubsub is not
//! started. This mirrors `.github/workflows/ci.yml` (`llm-router-integration`,
//! `provider-contract`), which set `III_ENGINE_BIN` and build `state/`.
//!
//! Skip vs fail: a binary the fixture *found on its own* (`iii` on PATH, a
//! sibling `state/target/…` build, the newest cached Compose package) that
//! turns out unusable is a **skip with the reason and the process's stderr**.
//! A binary named *explicitly* through `III_ENGINE_BIN` / `III_STATE_BIN` that
//! fails is a **hard error** — the caller asked for exactly this run.
#![allow(dead_code, unused_macros)]

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Where a binary came from: an explicit env var is a request that must be
/// honored; anything the fixture discovered by itself is best effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Explicit(&'static str),
    Discovered,
}

pub struct Engine {
    pub url: String,
    pub child: Child,
    pub state_child: Option<Child>,
    pub dir: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(mut s) = self.state_child.take() {
            let _ = s.kill();
            let _ = s.wait();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `III_ENGINE_BIN`, else `iii` on PATH.
pub fn engine_bin() -> Option<(PathBuf, Provenance)> {
    if let Some(p) = std::env::var_os("III_ENGINE_BIN") {
        return Some((p.into(), Provenance::Explicit("III_ENGINE_BIN")));
    }
    let on_path = Command::new("iii")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    on_path.then(|| ("iii".into(), Provenance::Discovered))
}

/// `III_STATE_BIN`, else the in-repo `state` build next to this crate
/// (`../state/target/{debug,release}/state`, what CI builds), else the newest
/// `state` package in the Compose cache (`~/.iii/compose/packages/state-*/`).
pub fn state_bin() -> Option<(PathBuf, Provenance)> {
    if let Some(p) = std::env::var_os("III_STATE_BIN") {
        return Some((p.into(), Provenance::Explicit("III_STATE_BIN")));
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for profile in ["debug", "release"] {
        let p = repo
            .join("state")
            .join("target")
            .join(profile)
            .join("state");
        if p.is_file() {
            return Some((p, Provenance::Discovered));
        }
    }
    let cache = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".iii/compose/packages");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&cache)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("state-"))
        })
        .map(|p| p.join("state"))
        .filter(|p| p.is_file())
        .collect();
    // lexical order is good enough to prefer the newest package build
    candidates.sort();
    candidates.pop().map(|p| (p, Provenance::Discovered))
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Per-test worker identity so parallel probes never collide on a name.
pub fn test_init_options() -> InitOptions {
    static NEXT_WORKER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut metadata = iii_sdk::runtime::WorkerMetadata::default();
    let worker_id = NEXT_WORKER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    metadata.name = format!("{}-test-{worker_id}", metadata.name);
    InitOptions {
        metadata: Some(metadata),
        ..InitOptions::default()
    }
}

/// stderr is drained on its own thread from the moment the child starts, so a
/// chatty engine can never block on a full pipe before it binds its port (which
/// would turn a healthy boot into a false "not ready"). Joined only on failure.
struct StderrDrain(Option<std::thread::JoinHandle<String>>);

impl StderrDrain {
    fn start(child: &mut Child) -> Self {
        let handle = child.stderr.take().map(|mut err| {
            std::thread::spawn(move || {
                let mut out = String::new();
                let _ = err.read_to_string(&mut out);
                out
            })
        });
        StderrDrain(handle)
    }

    /// Call after the child has been killed/exited (the pipe closes, the
    /// thread finishes). Bounded to keep failure output readable.
    fn collect(mut self) -> String {
        let out = self
            .0
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let out = out.trim();
        if out.is_empty() {
            "(no stderr)".to_string()
        } else {
            out.chars().take(2000).collect()
        }
    }
}

/// The one decision the whole fixture turns on: an explicitly requested
/// binary failing is the caller's error to see; a discovered one is a skip.
fn unusable(what: &str, bin: &Path, provenance: Provenance, detail: String) -> Option<Engine> {
    match provenance {
        Provenance::Explicit(var) => panic!(
            "{what} named by {var}={} is unusable: {detail}",
            bin.display()
        ),
        Provenance::Discovered => {
            eprintln!("skipping: {what} ({}) is unusable: {detail}", bin.display());
            None
        }
    }
}

/// Poll `probe` until it answers, or until the child exits / the deadline
/// passes. Ok(()) = ready; Err(detail) = not ready, with why.
async fn wait_ready<F, Fut>(child: &mut Child, what: &str, mut probe: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if probe().await {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("{what} exited with {status} before becoming ready"));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not become ready in {}s (still running)",
                READY_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Bare engine: `iii-worker-manager` on a free port and nothing else — the
/// shape every engine since 0.22 accepts in a direct `config.yaml`. Port is
/// pinned so parallel tests don't collide on the default.
pub async fn spawn_bare_engine() -> Option<Engine> {
    spawn_engine_with(|port, _dir| {
        format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
"#
        )
    })
    .await
}

/// Bare engine plus the standalone `state` worker (in-memory kv), polled
/// until `state::get` answers. None = skip (reason already printed).
pub async fn spawn_engine() -> Option<Engine> {
    let (state_bin, state_from) = state_bin().or_else(|| {
        eprintln!(
            "skipping: no state worker (set III_STATE_BIN, build state/ with `cargo build --manifest-path state/Cargo.toml --bin state`, or install any iii project once so the package cache holds one)"
        );
        None
    })?;
    let mut engine = spawn_bare_engine().await?;
    let state_config = engine.dir.join("state-config.yaml");
    std::fs::write(
        &state_config,
        "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
    )
    .expect("write state config");
    let mut state_child = match Command::new(&state_bin)
        .arg("--url")
        .arg(&engine.url)
        .arg("--config")
        .arg(&state_config)
        .current_dir(&engine.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return unusable("state worker", &state_bin, state_from, e.to_string()),
    };
    let stderr = StderrDrain::start(&mut state_child);
    let probe = register_worker(&engine.url, test_init_options());
    let ready = wait_ready(&mut state_child, "state worker", || {
        let probe = probe.clone();
        async move {
            probe
                .trigger(TriggerRequest {
                    function_id: "state::get".into(),
                    payload: json!({ "scope": "engine-fixture", "key": "ready" }),
                    action: None,
                    timeout_ms: Some(1000),
                })
                .await
                .is_ok()
        }
    })
    .await;
    probe.shutdown();
    match ready {
        Ok(()) => {
            engine.state_child = Some(state_child);
            Some(engine)
        }
        Err(detail) => {
            let _ = state_child.kill();
            let _ = state_child.wait();
            let detail = format!("{detail}; stderr:\n{}", stderr.collect());
            unusable("state worker", &state_bin, state_from, detail)
        }
    }
}

/// Engine bootstrap: free port + temp dir, the config the caller composes,
/// spawn, poll `engine::workers::list` until it answers.
pub async fn spawn_engine_with(config_for: impl FnOnce(u16, &Path) -> String) -> Option<Engine> {
    let (bin, from) = engine_bin().or_else(|| {
        eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
        None
    })?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!(
        "{}-it-{}",
        env!("CARGO_PKG_NAME"),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let config = config_for(port, &dir);
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .expect("write config");
    let mut child = match Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return unusable("iii engine", &bin, from, e.to_string());
        }
    };
    let stderr = StderrDrain::start(&mut child);
    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, test_init_options());
    let ready = wait_ready(&mut child, "iii engine", || {
        let probe = probe.clone();
        async move {
            probe
                .trigger(TriggerRequest {
                    function_id: "engine::workers::list".into(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(1000),
                })
                .await
                .is_ok()
        }
    })
    .await;
    probe.shutdown();
    match ready {
        Ok(()) => Some(Engine {
            url,
            child,
            state_child: None,
            dir,
        }),
        Err(detail) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            let detail = format!("{detail}; stderr:\n{}", stderr.collect());
            unusable("iii engine", &bin, from, detail)
        }
    }
}

/// `let engine = engine_or_skip!();` — engine + state, or return from the
/// test with the reason on stderr.
#[macro_export]
macro_rules! engine_or_skip {
    () => {
        match $crate::engine_fixture::spawn_engine().await {
            Some(e) => e,
            None => return,
        }
    };
}

/// Same, engine only (no state worker).
#[macro_export]
macro_rules! bare_engine_or_skip {
    () => {
        match $crate::engine_fixture::spawn_bare_engine().await {
            Some(e) => e,
            None => return,
        }
    };
}
