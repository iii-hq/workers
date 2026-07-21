use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use iii_cron::config::CronConfig;
use iii_cron::configuration;

#[derive(Parser, Debug)]
#[command(name = "cron", about = "Cron scheduler worker for iii.")]
struct Cli {
    /// Optional seed config.yaml. This seeds the configuration entry only when
    /// nothing is stored yet; thereafter the configuration worker is the
    /// authoritative source and `--config` is ignored for the stored value.
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
        name: "cron".to_string(),
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
            serde_json::to_string_pretty(&iii_cron::manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let seed: Option<CronConfig> = match cli.config.as_deref() {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_yaml::from_str::<CronConfig>(&s).map_err(|e| e.to_string()))
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
        .context("registering cron configuration schema")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading cron configuration")?;

    let boot = iii_cron::boot::start(iii.clone(), config).await?;
    tracing::info!("iii-cron ready");

    configuration::register_config_trigger(&iii, boot.parts())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-cron shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
