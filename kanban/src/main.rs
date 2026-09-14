//! `kanban` binary entry.
//!
//! Boot: parse the CLI, connect to the engine, register the trigger types and
//! the `kanban::*` functions, register the `kanban` configuration entry and
//! load its authoritative value, bind the reload trigger, inject the console
//! UI, then sleep until SIGINT/SIGTERM. The optional `--config` YAML seeds the
//! configuration entry the first time only; afterwards the configuration
//! worker is the source of truth.

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};

use iii_kanban::events::Subscribers;
use iii_kanban::functions::{self, Ctx};
use iii_kanban::store::BoardStore;
use iii_kanban::{configuration, ui};

#[derive(Parser, Debug)]
#[command(name = "kanban", about = "Kanban board worker for iii.")]
struct Cli {
    /// Optional seed config.yaml, used only when no `kanban` configuration is
    /// stored yet.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "kanban".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

fn read_seed(path: &str) -> Option<serde_json::Value> {
    match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_yaml::from_str::<serde_json::Value>(&raw).map_err(|e| e.to_string()))
    {
        Ok(value) => {
            tracing::info!(path, "loaded seed config");
            Some(value)
        }
        Err(error) => {
            tracing::warn!(
                path,
                error,
                "failed to load --config seed; continuing without it"
            );
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let seed = cli.config.as_deref().and_then(read_seed);

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let config = Arc::new(RwLock::new(iii_kanban::config::KanbanConfig::default()));
    let subscribers = Subscribers::default();
    iii_kanban::events::register_trigger_types(&iii, &subscribers);

    let ctx = Arc::new(Ctx {
        iii: iii.clone(),
        store: BoardStore::new(config.clone()),
        config: config.clone(),
        subscribers,
    });
    functions::register_functions(&ctx);

    configuration::register(&iii, seed)
        .await
        .map_err(anyhow::Error::msg)
        .context("registering kanban configuration schema")?;
    let reload = configuration::bind_reload(&iii, config.clone())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;
    // Closes the boot gap: fetch the authoritative value through the same
    // serialized path every later update takes.
    reload.run().await;

    ui::register(&iii);

    let board_file = iii_kanban::store::config_snapshot(&config).board_file();
    tracing::info!(board_file = %board_file.display(), "kanban ready");

    wait_for_shutdown_signal().await?;
    tracing::info!("kanban shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// Ctrl+C or SIGTERM — compose stops workers with SIGTERM, so it must reach
/// the clean shutdown too.
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
