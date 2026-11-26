//! `iii-cloud` binary entry: connect, seed and fetch the configuration, observe
//! its changes, register the `iii-cloud::*` surface and the console page, then
//! wait for shutdown.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_cloud::config::Config;
use iii_cloud::configuration;
use iii_cloud::ctx::{Ctx, IiiPlatform};
use iii_cloud::store::Store;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(
    name = "iii-cloud",
    about = "Take a project to iii cloud from your harness"
)]
struct Cli {
    /// Seed config registered as `initial_value` with the `configuration`
    /// worker on first registration. The live value is authoritative afterwards.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> Result<()> {
    // SAFETY: the first action of the synchronous main, before any runtime or
    // thread exists. `secret::provide` receives the person's secret value, and
    // the SDK would record every input on its span (`iii.payload.json`), which
    // `.observable(false)` does not stop; same as the secrets worker.
    unsafe { iii_cloud::disable_trace_payloads() };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: iii_cloud::WORKER_NAME.to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some("Take a project to iii cloud from your harness.".to_string()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let seed = match Config::load(&cli.config) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!(path = %cli.config, error = %e, "failed to load seed config; relying on the configuration worker");
            None
        }
    };
    if let Err(e) = configuration::register_config(&iii, seed.as_ref()).await {
        tracing::warn!(error = %e, "configuration::register failed; continuing with the seed");
    }
    let cfg = match configuration::fetch_config(&iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "configuration fetch failed; using seed/default config");
            seed.clone().unwrap_or_default()
        }
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let cell: configuration::ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    configuration::observe_config_changes(&iii, cell.clone());
    configuration::reconcile(&iii, &cell).await;

    let ctx = Ctx::new(cell, store, Arc::new(IiiPlatform { iii: iii.clone() }));
    // The runs left `running` are closed first; the clean-up of what they left
    // (docker, `iii kill`) goes on after the functions are registered, so a hung
    // docker daemon cannot keep them unregistered.
    let stale = iii_cloud::run::recover_runs(&ctx).await;
    iii_cloud::connect::resume(&ctx);
    iii_cloud::handoff::resume(&ctx);
    iii_cloud::functions::register_all(&iii, &ctx);
    tokio::spawn(iii_cloud::run::clean_stale(stale));
    iii_cloud::ui::register(&iii);
    // Resume the deploys that were still running, and follow pushes made elsewhere.
    iii_cloud::watch::ensure_poller(&ctx);
    tracing::info!("iii-cloud worker ready");

    wait_for_shutdown_signal().await?;
    tracing::info!("iii-cloud worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
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
