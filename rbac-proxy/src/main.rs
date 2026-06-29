//! `rbac-proxy` worker entry point.
//!
//! Boot order (binary-worker SOP §4c; spec *Boot order*):
//!   1. parse CLI (`--url` engine seed, `--config` optional one-time seed, `--manifest`)
//!   2. `register_worker(--url)`               — control connection
//!   3. `register_config(seed)` + `fetch_config()`   — REQUIRED boot dependency (fatal)
//!   4. build `ProxyState`; bind the public listener on `host:port`
//!   5. register `rbac-proxy::status`; start the catalog-cache feed
//!   6. (the axum server is spawned by the bind in step 4)
//!   7. `register_config_trigger`              — LAST, so the handler closes over fully-built state
//!   8. wait for SIGINT/SIGTERM → drain → `iii.shutdown_async()`

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use rbac_proxy::config::{self, WorkerConfig};
use rbac_proxy::configuration::{self, ConfigCell};
use rbac_proxy::server::{ProxyState, ServerHandle};
use rbac_proxy::{functions, manifest, redact_url};

#[derive(Parser, Debug)]
#[command(
    name = "rbac-proxy",
    about = "RBAC boundary proxy for the iii worker protocol — auth, gating, namespacing, middleware, and engine:: result filtering on its own port."
)]
struct Cli {
    /// iii engine WebSocket URL for the control connection (and the default
    /// upstream for the data plane).
    #[arg(long, default_value = config::DEFAULT_ENGINE_URL)]
    url: String,

    /// Optional one-time seed config (YAML) installed as the `configuration`
    /// entry's `initial_value` when none exists yet.
    #[arg(long)]
    config: Option<String>,

    /// Print the publish manifest as JSON and exit (no engine connection).
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    // 2. Control connection.
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "rbac-proxy".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // 3. Register schema + fetch authoritative config (REQUIRED).
    let seed = match cli.config.as_deref() {
        Some(path) => match load_seed(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load seed config; relying on the stored configuration entry");
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, &cli.url, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering rbac-proxy configuration schema")?;
    let cfg = configuration::fetch_config(&iii, &cli.url)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading rbac-proxy configuration")?;

    tracing::info!(
        host = %cfg.host,
        port = cfg.port,
        engine_url = %redact_url(&cfg.engine_url),
        rbac_enabled = cfg.rbac_enabled(),
        "starting rbac-proxy"
    );

    // 4. Build state + bind the public listener.
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let state = ProxyState::new(cell.clone(), iii.clone());
    let server = Arc::new(
        ServerHandle::bind_and_serve(state.clone(), &cfg.host, cfg.port)
            .await
            .context("binding the public RBAC listener")?,
    );

    // 5. Public function + catalog-cache feed.
    functions::register_all(&iii, &state);
    configuration::bind_catalog_refresh(&iii, state.catalog.clone());

    // 7. Config-change trigger LAST (closes over the fully-built server handle).
    configuration::register_config_trigger(&iii, cell, server.clone(), cli.url.clone())
        .context("registering the configuration change trigger")?;

    tracing::info!("rbac-proxy ready");

    // 8. Drain on signal.
    wait_for_shutdown_signal().await?;
    tracing::info!("rbac-proxy shutting down");
    server.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}

/// Load a one-time seed config from a YAML file.
fn load_seed(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let cfg: WorkerConfig =
        serde_yaml::from_str(&contents).with_context(|| format!("parsing {path}"))?;
    Ok(cfg)
}

/// Wait for SIGINT or, on Unix, SIGTERM (so `docker stop` / `kubectl delete`
/// trigger a graceful shutdown, not just Ctrl-C).
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
