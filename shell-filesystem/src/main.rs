use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod config;
mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "shell-filesystem",
    about = "Sandboxed filesystem operations on the iii bus (shell::filesystem::*)"
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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let mut cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %cli.config,
                "failed to load config, using defaults"
            );
            config::WorkerConfig::default()
        }
    };

    if let Ok(raw) = std::env::var("SHELL_FILESYSTEM_MAX_INLINE_BYTES") {
        if let Ok(n) = raw.trim().parse::<usize>() {
            cfg.max_inline_bytes = n;
        }
    }

    let cfg = Arc::new(cfg);
    let read_config = shell_filesystem::ops::read::ReadConfig {
        max_inline_bytes: cfg.max_inline_bytes,
    };

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "shell-filesystem".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    shell_filesystem::register_with_config(&iii, read_config)
        .await
        .map_err(|e| anyhow::anyhow!("shell-filesystem register failed: {e}"))?;

    tracing::info!(
        max_inline_bytes = cfg.max_inline_bytes,
        "shell-filesystem registered (shell::filesystem::*)"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("shell-filesystem shutting down");
    iii.shutdown_async().await;
    Ok(())
}
