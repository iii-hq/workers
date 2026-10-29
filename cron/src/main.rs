use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

#[derive(Parser, Debug)]
#[command(name = "cron", about = "Cron scheduler worker for iii.")]
struct Cli {
    /// The engine passes a worker's `config:` block as `--config <file>`. The
    /// cron worker has no settings and does not read the file.
    #[arg(long, hide = true)]
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
    if let Some(path) = cli.config.as_deref() {
        tracing::info!(path = %path, "the cron worker has no settings; --config is not read");
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let boot = iii_cron::boot::start(iii.clone());
    tracing::info!("iii-cron ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-cron shutting down");
    boot.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}
