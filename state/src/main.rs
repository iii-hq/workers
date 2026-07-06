//! `iii-state` binary entry.
//!
//! Boot: parse CLI, connect to the engine, register the `state` configuration
//! schema, fetch the authoritative value, start the trigger type + functions
//! via `boot::start`, wire the `configuration:updated` reload trigger, then
//! sleep until Ctrl+C. The optional `--config` YAML file is a **seed only**:
//! it is used to populate the configuration entry the first time, after which
//! the configuration worker is the source of truth.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};

use iii_state::config::StateConfig;
use iii_state::configuration;

#[derive(Parser, Debug)]
#[command(name = "state", about = "State worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. This seeds the configuration entry only when
    /// nothing is stored yet; thereafter the configuration worker is the
    /// authoritative source and `--config` is ignored for the stored value.
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
        name: "state".to_string(),
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
            serde_json::to_string_pretty(&iii_state::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    // Parse the optional --config seed (best-effort). It only seeds the
    // configuration entry on first boot; the authoritative value comes from the
    // configuration worker below.
    let seed: Option<StateConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<StateConfig>(&s).map_err(|e| e.to_string()))
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

    // Register the schema (seeding initial_value only when nothing is stored),
    // then load the authoritative config the worker actually starts from.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering state configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading state configuration")?;

    let boot = iii_state::boot::start(iii.clone(), config.clone()).await?;
    tracing::info!(adapter = %config.effective_adapter_name(), "iii-state ready");

    // Subscribe to configuration:updated so triggers_enabled/max_value_bytes
    // reload live and a save_interval_ms change respawns the adapter's save
    // loop (see configuration).
    configuration::register_config_trigger(&iii, boot.ctx.clone(), boot.apply_lock.clone())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-state shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
