//! `github` binary entry: connect, seed + fetch config, bind the
//! configuration-change trigger, register the `github::*` surface, then wait
//! for shutdown. No gh probe at boot — a missing gh binary surfaces at call
//! time as `gh_not_found`, so interface collection works on bare runners.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use github::config::Config;
use github::configuration;
use github::functions::register_all;

#[derive(Parser, Debug)]
#[command(name = "github", about = "GitHub CLI (gh) as an iii worker")]
struct Cli {
    /// Seed config registered as `initial_value` with the `configuration`
    /// worker on first registration. Defaults to ./config.yaml. The live value
    /// from the configuration worker is authoritative once an entry exists.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
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
    tracing::info!(url = %cli.url, seed_config = %cli.config, "connecting to III engine");

    // Arc-wrapped for `ui::register` (the console-ui crate clones the client
    // into its hot-reload watcher task); everything else auto-derefs.
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "github".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some("GitHub CLI (gh) as an iii worker.".to_string()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // Seed from config.yaml when present; a missing file means defaults.
    let seed = match Config::load(&cli.config) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!(path = %cli.config, error = %e, "failed to load seed config; relying on the configuration worker");
            None
        }
    };

    // Config registration is best-effort: if the configuration worker is
    // unreachable (or absent, as in interface collection on a bare engine) the
    // worker still serves with the seed / built-in defaults. Registering the
    // github::* surface must not depend on the configuration worker being up.
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

    let cell: configuration::ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    if let Err(e) = configuration::register_config_trigger(&iii, cell.clone()) {
        tracing::warn!(error = %e, "configuration change trigger registration failed");
    }
    configuration::reconcile(&iii, &cell).await;

    // Register the `github::called` trigger type first so the emitter threaded
    // through every function has a live subscriber set to fan out to.
    let called = github::events::register_called_trigger(&iii);
    register_all(&iii, &cell, &called);

    // Injectable console UI — after the github::* functions so the console can
    // attribute the assets.
    github::ui::register(&iii);

    tracing::info!("github worker ready: 30 typed functions + github::exec + github::api");

    wait_for_shutdown_signal().await?;
    tracing::info!("github worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

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
