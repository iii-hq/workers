use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use context_compaction::{manifest, register_with_iii};
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

#[derive(Parser, Debug)]
#[command(
    name = "context-compaction",
    about = "Out-of-band session-history compactor. Subscribes to agent::events::TurnEnd and writes a session-tree Compaction entry when the running token count crosses the configured threshold."
)]
struct Cli {
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
        let m = manifest::build();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "context-compaction".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = register_with_iii(&iii)?;
    tracing::info!(
        threshold = context_compaction::config::trigger_tokens(),
        keep_recent = context_compaction::config::keep_recent_turns(),
        "context-compaction ready, subscribed to agent::events"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("context-compaction shutting down");
    iii.shutdown_async().await;
    Ok(())
}
