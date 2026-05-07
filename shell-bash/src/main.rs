use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod config;
mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "shell-bash",
    about = "Sandboxed shell execution on the iii bus under shell::bash::*."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
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
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c.apply_env_overrides(),
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::WorkerConfig::default().apply_env_overrides()
        }
    };

    let exec_cfg = cfg.exec_config();

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "shell-bash".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    shell_bash::register_with_config(iii.as_ref(), exec_cfg)
        .await
        .context("shell-bash register failed")?;

    tracing::info!(
        default_timeout_ms = exec_cfg.default_timeout_ms,
        trigger_timeout_ms = exec_cfg.trigger_timeout_ms,
        max_output_bytes = exec_cfg.max_output_bytes,
        "shell-bash ready (shell::bash::exec | shell::bash::which | shell::bash::detect_clis)",
    );

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shell-bash shutting down");
    iii.shutdown_async().await;
    Ok(())
}
