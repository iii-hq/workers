use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod config;
mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "iii-harness",
    about = "Meta-worker that composes the modular workers backing the iii chat surface."
)]
struct Cli {
    #[arg(long, env = "III_HARNESS_CONFIG", default_value = "./config.yaml")]
    config: String,

    /// Engine WebSocket URL. Falls back to `engine_url` in `--config` when unset.
    #[arg(long, env = "III_URL")]
    url: Option<String>,

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
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::WorkerConfig::default()
        }
    };

    let url = cli.url.unwrap_or_else(|| cfg.engine_url.clone());

    let iii = register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "harness".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = harness::register_with_iii_with_engine_url(&iii, &url).await?;
    tracing::info!(
        "harness ready — registered harness::status; expecting {} runtime workers from iii.worker.yaml",
        harness::EXPECTED_WORKERS.len()
    );

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("harness shutting down");
    iii.shutdown_async().await;
    Ok(())
}
