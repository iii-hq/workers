//! Boot the laya provider: fetch the checkpoint, load it, register on the bus.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_laya::{configuration, download, engine, register, LayaClient};
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
    /// Use a local encoder GGUF for the default model instead of the Hub.
    #[arg(long, env = "III_LAYA_ENCODER_GGUF")]
    encoder_gguf: Option<PathBuf>,
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
    let (model, revision, preload) = (
        initial.model.clone(),
        initial.revision.clone(),
        initial.preload.clone(),
    );
    let options = engine::Options {
        threads: initial.threads,
        gpu_layers: initial.gpu_layers,
        batch_rows: initial.batch_questions,
    };
    // candle (the head) reads RAYON_NUM_THREADS per call; an operator export wins.
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        std::env::set_var("RAYON_NUM_THREADS", initial.threads.to_string());
    }
    let config = configuration::new_cell(initial);
    // Functions register only once the model answers: until then the hub
    // reports provider_unavailable, which is the honest state.
    let client = tokio::task::spawn_blocking(move || -> anyhow::Result<LayaClient> {
        let mut checkpoints = Vec::new();
        for (i, name) in std::iter::once(&model).chain(&preload).enumerate() {
            let checkpoint = match &cli.checkpoint_dir {
                Some(dir) if i == 0 => download::local(name, dir)?,
                // A local directory holds one checkpoint; extra ones need the Hub.
                Some(_) => {
                    tracing::warn!(model = name, "preload is ignored with --checkpoint-dir");
                    continue;
                }
                None => {
                    tracing::info!(
                        model = name,
                        "fetching laya checkpoint from the Hugging Face Hub"
                    );
                    download::fetch(
                        name,
                        revision.as_deref(),
                        cli.encoder_gguf.as_deref().filter(|_| i == 0),
                    )?
                }
            };
            tracing::info!(
                model = checkpoint.model,
                revision = checkpoint.revision,
                "loading laya checkpoint"
            );
            checkpoints.push(checkpoint);
        }
        LayaClient::load(&checkpoints, options)
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
