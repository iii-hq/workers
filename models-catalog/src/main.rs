use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "models-catalog",
    about = "Model capabilities knowledge base (models::*) on the iii bus."
)]
struct Cli {
    #[arg(
        long,
        env = "III_MODELS_CATALOG_CONFIG",
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

    let cfg = match models_catalog::config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            models_catalog::config::ModelsCatalogConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let url = cli.url.clone().unwrap_or_else(|| cfg.engine_url.clone());

    let iii = register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "models-catalog".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = models_catalog::register_with_iii(&iii, &cfg)
        .await
        .context("models-catalog register failed")?;
    tracing::info!("models-catalog registered (models::*)");

    wait_for_shutdown().await?;

    tracing::info!("models-catalog shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}

