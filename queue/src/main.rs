//! `queue` binary entry.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_queue::config::QueueConfig;
use iii_queue::configuration;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

#[derive(Parser, Debug)]
#[command(name = "queue", about = "Durable queue worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. This seeds the configuration entry only when
    /// nothing is stored yet; thereafter the configuration worker is
    /// authoritative.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "queue".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    use tracing_subscriber::prelude::*;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(iii_queue::observability::otel_layer())
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&iii_queue::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let seed: Option<QueueConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<QueueConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(config) => {
                tracing::info!(path = %path, "loaded seed config");
                Some(config.normalized())
            }
            Err(err) => {
                tracing::warn!(path = %path, error = %err, "failed to load --config seed; continuing without seed");
                None
            }
        },
        None => None,
    };

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering queue configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading queue configuration")?;
    if let Some(seed) = seed.as_ref() {
        if seed.adapter != config.adapter {
            tracing::warn!(
                seed_adapter = ?seed.adapter,
                stored_adapter = ?config.adapter,
                "--config seed adapter ignored; the stored configuration is authoritative"
            );
        }
    }

    let boot = iii_queue::boot::start(iii.clone(), config).await?;
    configuration::register_config_trigger(
        &iii,
        boot.runtime.clone(),
        boot.trigger_handler.clone(),
    )
    .map_err(anyhow::Error::msg)
    .context("binding queue configuration trigger")?;

    // Reconcile once after binding the trigger so an update made between the
    // initial fetch and trigger registration cannot be missed indefinitely.
    boot.runtime
        .refresh_config(&boot.trigger_handler)
        .await
        .context("reconciling queue configuration after trigger binding")?;

    tracing::info!("queue worker ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("queue worker shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
