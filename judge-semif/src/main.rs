//! Boot the SemIf provider: fetch the GGUF, load it, register on the bus.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_semif::{configuration, download, engine, register, SemifClient};
use std::{path::PathBuf, sync::Arc};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "judge-semif",
    about = "SemIf provider for the judge hub: typed Noul, Choice and Score decisions from a frozen GGUF LLM running in-process."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Use a local GGUF file instead of the Hugging Face Hub.
    #[arg(long, env = "III_SEMIF_GGUF")]
    gguf: Option<PathBuf>,
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
                name: "judge-semif".into(),
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
    let model = initial.model.clone();
    let options = engine::Options {
        threads: initial.threads,
        gpu_layers: initial.gpu_layers,
        context_tokens: initial.context_tokens,
    };
    let config = configuration::new_cell(initial);
    // Functions register only once the model answers: until then the hub
    // reports provider_unavailable, which is the honest state.
    let client = tokio::task::spawn_blocking(move || -> anyhow::Result<SemifClient> {
        let checkpoint = match &cli.gguf {
            Some(path) => download::local(&model, path)?,
            None => {
                tracing::info!(model, "fetching the SemIf GGUF from the Hugging Face Hub");
                download::fetch(&model)?
            }
        };
        tracing::info!(
            model = checkpoint.model,
            revision = checkpoint.revision,
            "loading the SemIf GGUF"
        );
        let client = SemifClient::load(&checkpoint, options)?;
        tracing::info!(device = client.device(), "selected inference device");
        Ok(client)
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
    tracing::info!("judge-semif ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("judge-semif ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
