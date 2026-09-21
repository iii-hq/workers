//! Boot the generic JEV worker using the shared configuration-client lifecycle.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use jev::{configuration, manifest, register, JevClient, JevConfig};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "jev", about = manifest::DESCRIPTION)]
struct Cli {
    /// One-time YAML/JSON seed, used only if no authoritative config exists.
    #[arg(long)]
    config: Option<String>,
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let client = JevClient::new(std::env::var("TYPESAFE_API_KEY").ok());
    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match JevConfig::from_file(path) {
            Ok(seed) => Some(seed),
            Err(_) => {
                tracing::warn!(
                    "Cannot load JEV config seed; using authoritative config or defaults"
                );
                None
            }
        });
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "jev".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)?;
    let config = configuration::new_cell(
        configuration::fetch_config(&iii)
            .await
            .map_err(anyhow::Error::msg)?,
    );
    register(&iii, config.clone(), client);
    #[cfg(feature = "console-ui")]
    register::register_console_ui(&iii);
    configuration::register_config_trigger(&iii, config)?
        .run()
        .await;
    let result = wait_for_shutdown().await;
    // shutdown_async only signals the SDK's dedicated connection thread. Join
    // it before main returns so pending telemetry can finish flushing.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}
#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tracing::info!("JEV worker ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("JEV worker ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
