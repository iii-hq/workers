//! Boot the judge hub using the shared configuration-client lifecycle.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge::{configuration, register, validate_provider, JudgeConfig, DEFAULT_PROVIDER};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "judge",
    about = "Provider-neutral Noul, Choice and Score evaluations: judge::* forwarded to the selected judge-<provider> worker."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// `judge-<provider>` worker seeded into the configuration entry on first
    /// boot; the stored value (Console Settings → Workers → judge) wins afterwards.
    #[arg(long, env = "JUDGE_PROVIDER", default_value = DEFAULT_PROVIDER)]
    provider: String,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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
    let seed = JudgeConfig {
        provider: cli.provider,
    };
    configuration::register_config(&iii, Some(&seed))
        .await
        .map_err(anyhow::Error::msg)?;
    let config = configuration::new_cell(
        configuration::fetch_config(&iii)
            .await
            .map_err(anyhow::Error::msg)?,
    );
    register(&iii, config.clone());
    #[cfg(feature = "console-ui")]
    judge::ui::register(&iii);
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
    tracing::info!("judge hub ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("judge hub ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
