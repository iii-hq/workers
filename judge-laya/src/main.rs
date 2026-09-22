//! Boot the laya provider: fetch the checkpoint, load it, register on the bus.
use candle_core::Device;
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_laya::{configuration, download, register, LayaClient};
use std::{path::PathBuf, sync::Arc};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "judge-laya",
    about = "laya provider for the judge hub: typed Noul, Choice and Score decisions from a ModernBERT checkpoint running in-process."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Use a local checkpoint directory (model.safetensors, encoder/config.json,
    /// rl_agent_config.json, tokenizer.json) instead of the Hugging Face Hub.
    #[arg(long, env = "III_LAYA_CHECKPOINT_DIR")]
    checkpoint_dir: Option<PathBuf>,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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
                name: "judge-laya".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    configuration::register_config(&iii, None)
        .await
        .map_err(anyhow::Error::msg)?;
    let initial = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)?;
    let (model, revision) = (initial.model.clone(), initial.revision.clone());
    let config = configuration::new_cell(initial);
    // Functions register only once the model answers: until then the hub
    // reports provider_unavailable, which is the honest state.
    let client = tokio::task::spawn_blocking(move || -> anyhow::Result<LayaClient> {
        let checkpoint = match cli.checkpoint_dir {
            Some(dir) => download::local(&model, &dir)?,
            None => {
                tracing::info!(model, "fetching laya checkpoint from the Hugging Face Hub");
                download::fetch(&model, revision.as_deref())?
            }
        };
        tracing::info!(
            model = checkpoint.model,
            revision = checkpoint.revision,
            "loading laya checkpoint"
        );
        // ponytail: CPU only; a GPU device is a build feature to add when a deployment
        // needs it (candle's cuda feature breaks --all-features builds without a toolkit).
        LayaClient::load(&checkpoint, Device::Cpu)
    })
    .await??;
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
    tracing::info!("judge-laya ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("judge-laya ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
