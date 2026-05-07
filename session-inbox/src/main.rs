use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};
use session_inbox::{config, manifest, register_with_iii};

#[derive(Parser, Debug)]
#[command(
    name = "session-inbox",
    about = "Per-session inbox: push, drain, peek backed by iii state."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// Engine WebSocket URL. Overrides `engine_url` in `--config` when set (or use `III_URL`).
    #[arg(long, env = "III_URL")]
    url: Option<String>,

    #[arg(long)]
    manifest: bool,
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
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
    let cfg = Arc::new(cfg);

    let url = cli.url.unwrap_or_else(|| cfg.engine_url.clone());

    let iii = register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "session-inbox".to_string(),
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
    tracing::info!(
        "session-inbox ready ({}, {}, {})",
        session_inbox::PUSH_ID,
        session_inbox::DRAIN_ID,
        session_inbox::PEEK_ID
    );

    wait_for_shutdown().await;
    tracing::info!("session-inbox shutting down");
    iii.shutdown_async().await;
    Ok(())
}
