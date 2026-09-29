//! Boot the SemIf provider: register on the bus, then fetch and load the GGUF
//! on first use (or at once while the judge hub selects SemIf).
use clap::Parser;
use iii_llama_runtime::{ModelSlot, IDLE_RELEASE};
use iii_sdk::IIIClient;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_semif::{configuration, download, engine, register, SemifClient, PROVIDER};
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
    let (gguf, model) = (cli.gguf, initial.model.clone());
    let options = engine::Options {
        threads: initial.threads,
        gpu_layers: initial.gpu_layers,
        context_tokens: initial.context_tokens,
        parallel: initial.parallel_questions,
    };
    let config = configuration::new_cell(initial);
    #[cfg(feature = "console-ui")]
    register::register_console_ui(&iii);
    configuration::register_config_trigger(&iii, config.clone())?
        .run()
        .await;
    // The model loads on first use, or right away while the hub selects this
    // provider (see `serve`).
    let slot = ModelSlot::new(move || -> anyhow::Result<SemifClient> {
        let checkpoint = match &gguf {
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
    });
    register(&iii, config, slot.clone());
    tokio::spawn(slot.clone().release_idle(IDLE_RELEASE));
    let mut serving = tokio::spawn(serve(iii.clone(), slot));
    let result = tokio::select! {
        result = wait_for_shutdown() => result,
        served = &mut serving => match served? {
            // Pinned for good (the hub exposes no selection): serve until shutdown.
            Ok(()) => wait_for_shutdown().await,
            Err(error) => Err(error),
        },
    };
    serving.abort();
    // shutdown_async only signals the SDK's dedicated connection thread. Join
    // it before main returns so pending telemetry can finish flushing.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}
/// Keep the SemIf model loaded while the judge hub selects this provider (see
/// `ModelSlot::follow_selection`); a hub that does not expose its
/// configuration id pins it for good.
async fn serve(iii: Arc<IIIClient>, slot: Arc<ModelSlot<SemifClient>>) -> anyhow::Result<()> {
    match iii_config_client::follow(
        &iii,
        "judge",
        "judge-semif::on-judge-config-change",
        "Internal: keep the SemIf model loaded while the judge hub selects this provider.",
    )
    .await
    {
        Ok(hub) => slot.follow_selection(hub, PROVIDER).await,
        Err(reason) => {
            tracing::warn!(
                reason,
                "judge hub selection unknown; keeping the SemIf model loaded"
            );
            slot.pin(true);
            Ok(())
        }
    }
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
