//! `secrets` binary.
//!
//! Boot: before any thread exists, turn off SDK payload tracing and take
//! `III_SECRETS_KEY` out of the environment; connect; register the `secrets`
//! configuration entry and read it; open the vault; register the
//! `secrets::changed` trigger type and the `secrets::*` functions; follow
//! configuration changes; sleep until SIGINT/SIGTERM.
use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tracing_subscriber::EnvFilter;

use secrets::access::CallerDirectory;
use secrets::detect::Detector;
use secrets::events::Subscribers;
use secrets::keys::EnvKey;
use secrets::{configuration, Ctx, Store, StorePaths};

#[derive(Parser)]
#[command(
    name = "secrets",
    about = "Local protected secret store: secret://NAME references resolved only by allowlisted workers."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> anyhow::Result<()> {
    secrets::disable_trace_payloads();
    let env_key = EnvKey::take_from_process();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli, env_key))
}

async fn run(cli: Cli, env_key: EnvKey) -> anyhow::Result<()> {
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "secrets".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    configuration::register(&iii)
        .await
        .map_err(anyhow::Error::msg)?;
    let config = configuration::fetch(&iii)
        .await
        .map_err(anyhow::Error::msg)?;
    let store = Arc::new(Store::new(StorePaths::from_config(&config), env_key));
    let vault_path = store.vault_path().await;
    match store.open().await {
        Ok(count) => tracing::info!(vault = %vault_path.display(), count, "vault open"),
        // Keep serving: every call reports the problem until it is fixed,
        // and the file is never overwritten in the meantime.
        Err(error) => tracing::error!(error = %error, "vault is not usable"),
    }
    let ctx = Arc::new(Ctx {
        iii: iii.clone(),
        store: store.clone(),
        callers: CallerDirectory::new(iii.clone()),
        subscribers: Subscribers::default(),
        detector: Detector::from_env(),
    });
    secrets::register(&ctx);
    configuration::bind_reload(&iii, store)?.run().await;
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
    tracing::info!("secrets ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    tracing::info!("secrets shutting down");
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("secrets ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("secrets shutting down");
    Ok(())
}
