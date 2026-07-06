//! `iii-pubsub` binary entry.
//!
//! Boot: parse CLI, connect to the engine, register the `pubsub` configuration
//! schema, fetch the authoritative value, start the hub via `boot::start`,
//! wire the `configuration:updated` reload trigger, then sleep until Ctrl+C.
//! The optional `--config` YAML file is a seed only.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use iii_pubsub::config::PubSubConfig;
use iii_pubsub::configuration;

#[derive(Parser, Debug)]
#[command(name = "pubsub", about = "Pub/sub worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. Seeds the configuration entry only when
    /// nothing is stored yet; thereafter the configuration worker is the
    /// authoritative source.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "pubsub".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&iii_pubsub::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let seed: Option<PubSubConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<PubSubConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config");
                Some(c.normalized())
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load --config seed; continuing without seed");
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
        .context("registering pubsub configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading pubsub configuration")?;

    let boot = iii_pubsub::boot::start(iii.clone(), config).await?;
    tracing::info!(
        adapter = %boot.config.read().await.effective_adapter_name(),
        "iii-pubsub ready"
    );

    configuration::register_config_trigger(
        &iii,
        boot.hub.clone(),
        boot.invoker.clone(),
        boot.config.clone(),
        boot.apply_lock.clone(),
    )
    .map_err(anyhow::Error::msg)
    .context("binding configuration trigger")?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-pubsub shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
