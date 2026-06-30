//! `iii-http` binary entry.
//!
//! Boot: parse CLI, connect to the engine, start the HTTP server via
//! `boot::start`, then sleep until Ctrl+C. The authoritative configuration
//! worker integration lands in a later phase; for now an optional `--config`
//! YAML file (or the built-in defaults) seeds the server.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};

use iii_http::config::RestApiConfig;

#[derive(Parser, Debug)]
#[command(name = "iii-http", about = "HTTP server worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. The authoritative config worker integration
    /// arrives in a later phase; today this file (or defaults) seeds startup.
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&iii_http::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    // Seed config from --config (best-effort) or fall back to defaults.
    let config = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<RestApiConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config");
                c.normalized()
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load config; using defaults");
                RestApiConfig::default()
            }
        },
        None => RestApiConfig::default(),
    };

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let boot = iii_http::boot::start(iii.clone(), config).await?;
    tracing::info!(address = %boot.local_addr, "iii-http ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-http shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
