use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use hook_fanout::{config, register_with_iii, FUNCTION_ID};
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

#[derive(Parser, Debug)]
#[command(
    name = "hook-fanout",
    about = "Publish-collect primitive for iii hook topics — fans events to subscribers and merges replies."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

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
        let m = hook_fanout::build_manifest();
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
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "hook-fanout".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    register_with_iii(&iii, &cfg);
    tracing::info!(%FUNCTION_ID, "hook-fanout ready, waiting for invocations");

    tokio::signal::ctrl_c().await?;
    tracing::info!("hook-fanout shutting down");
    iii.shutdown_async().await;
    Ok(())
}
