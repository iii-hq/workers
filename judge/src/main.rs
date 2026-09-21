//! Boot the judge hub.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge::{manifest, register, validate_provider, DEFAULT_PROVIDER};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "judge", about = manifest::DESCRIPTION)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// `judge-<provider>` worker used when a request omits `provider`.
    #[arg(long, env = "JUDGE_PROVIDER", default_value = DEFAULT_PROVIDER)]
    provider: String,
    /// Print the registry manifest and exit without connecting.
    #[arg(long)]
    manifest: bool,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())?
        );
        return Ok(());
    }
    if validate_provider(&cli.provider).is_err() {
        anyhow::bail!(
            "provider must be a worker-name suffix: lowercase letters, digits and hyphens, at most 64 bytes"
        );
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "judge".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    register(&iii, &cli.provider);
    let result = wait_for_shutdown(&cli.provider).await;
    // shutdown_async only signals the SDK's dedicated connection thread. Join
    // it before main returns so pending telemetry can finish flushing.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}
#[cfg(unix)]
async fn wait_for_shutdown(provider: &str) -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tracing::info!(provider, "judge hub ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown(provider: &str) -> anyhow::Result<()> {
    tracing::info!(provider, "judge hub ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
