use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod config;
mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "iii-provider-router",
    about = "router::stream_assistant provider router plus router::abort and push helpers."
)]
struct Cli {
    #[arg(
        long,
        env = "III_PROVIDER_ROUTER_CONFIG",
        default_value = "./config.yaml"
    )]
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
                name: "provider-router".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    provider_router::register_with_iii(&iii)
        .await
        .context("provider-router register failed")?;

    tracing::info!(
        "provider-router registered (router::stream_assistant, router::abort, router::push_steering, router::push_followup)"
    );

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("provider-router shutting down");
    iii.shutdown_async().await;
    Ok(())
}
