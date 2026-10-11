//! `context-clm` binary: connect to the engine and serve until SIGINT/SIGTERM.
use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "context-clm",
    about = "Model-managed context for the harness: the model edits a mirror of its own context under a token budget."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> anyhow::Result<()> {
    // Handler payloads are whole transcripts; keep them out of span events.
    // Set before the runtime spawns any thread.
    std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1");
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: context_clm::WORKER_NAME.into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let result = wait_for_shutdown().await;
    // shutdown() joins the SDK's connection thread so pending telemetry can
    // flush before main returns.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tracing::info!("context-clm ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    tracing::info!("context-clm shutting down");
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("context-clm ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("context-clm shutting down");
    Ok(())
}
