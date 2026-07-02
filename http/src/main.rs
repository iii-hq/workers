//! `iii-http` binary entry.
//!
//! Boot: parse CLI, connect to the engine, register the `http` configuration
//! schema, fetch the authoritative value, start the HTTP server via
//! `boot::start`, wire the `configuration:updated` reload trigger, then sleep
//! until Ctrl+C. The optional `--config` YAML file is a **seed only**: it is
//! used to populate the configuration entry the first time, after which the
//! configuration worker is the source of truth.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};

use iii_http::config::RestApiConfig;
use iii_http::configuration;

#[derive(Parser, Debug)]
#[command(name = "iii-http", about = "HTTP server worker for iii.")]
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
        name: "http".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Subscriber = fmt (human logs) + the tracing->OTel bridge, so the
    // per-request `HTTP` spans emitted in `handler.rs` reach the global OTel SDK
    // and `Span::set_parent` resolves (no `SetParentError::LayerNotFound`). The
    // bridge tracer re-reads the global provider per span (see
    // `iii_http::observability`), so installing it here -- before the SDK's async
    // `init_otel` sets the provider -- is safe: requests (and thus spans) only
    // happen after `boot::start` below, by which point the provider is live.
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(iii_http::observability::otel_layer())
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&iii_http::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    // Parse the optional --config seed (best-effort). It only seeds the
    // configuration entry on first boot; the authoritative value comes from the
    // configuration worker below.
    let seed: Option<RestApiConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<RestApiConfig>(&s).map_err(|e| e.to_string()))
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
    // then load the authoritative config the server actually binds from.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering http configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading http configuration")?;

    let boot = iii_http::boot::start(iii.clone(), config).await?;
    tracing::info!(address = %boot.local_addr, "iii-http ready");

    // Subscribe to configuration:updated so middleware/default_timeout and the
    // CORS/timeout/concurrency layers reload live on a same-address change, and
    // a host/port change rebinds the listener live (see configuration).
    configuration::register_config_trigger(
        &iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    )
    .map_err(anyhow::Error::msg)
    .context("binding configuration trigger")?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-http shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
