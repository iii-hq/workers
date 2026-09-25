//! Boot the decider provider: fetch the GGUF, load it, register on the bus.
use clap::Parser;
use iii_sdk::IIIClient;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_decider::{
    configuration, download, engine, register, DeciderClient, SharedConfig, PROVIDER,
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "judge-decider",
    about = "decider provider for the judge hub: typed Noul, Choice and Score decisions from decider-4b (GGUF) running in-process."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Use a local GGUF file instead of the Hugging Face Hub.
    #[arg(long, env = "III_DECIDER_GGUF")]
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
                name: "judge-decider".into(),
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
        parallel: initial.parallel_questions,
    };
    let config = configuration::new_cell(initial);
    #[cfg(feature = "console-ui")]
    register::register_console_ui(&iii);
    configuration::register_config_trigger(&iii, config.clone())?
        .run()
        .await;
    let mut serving = tokio::spawn(serve(iii.clone(), config, cli.gguf, model, options));
    let result = tokio::select! {
        result = wait_for_shutdown() => result,
        served = &mut serving => match served? {
            // Loaded for good (the hub exposes no selection): serve until shutdown.
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
/// Load the model and register the judge functions only while the judge hub's
/// default provider is decider or it preloads every provider (`preload_all`);
/// release both when neither holds. Functions
/// register only once the model answers: until then the hub reports
/// provider_unavailable, which is the honest state. A hub that does not expose
/// its configuration id leaves the model loaded for good.
async fn serve(
    iii: Arc<IIIClient>,
    config: SharedConfig,
    gguf: Option<PathBuf>,
    model: String,
    options: engine::Options,
) -> anyhow::Result<()> {
    // Loaded while the hub routes its default here or keeps every local model loaded.
    let selected = |hub: &Option<Value>| {
        hub.as_ref().is_some_and(|hub| {
            hub.get("provider").and_then(Value::as_str) == Some(PROVIDER)
                || hub.get("preload_all").and_then(Value::as_bool) == Some(true)
        })
    };
    let mut hub = match iii_config_client::follow(
        &iii,
        "judge",
        "judge-decider::on-judge-config-change",
        "Internal: load or release the decider model when the judge hub's default provider changes.",
    )
    .await
    {
        Ok(hub) => Some(hub),
        Err(reason) => {
            tracing::warn!(
                reason,
                "judge hub selection unknown; loading the decider model"
            );
            None
        }
    };
    loop {
        if let Some(hub) = &mut hub {
            if !selected(&hub.borrow()) {
                tracing::info!(
                    "the judge hub neither defaults to decider nor preloads every provider; the model stays unloaded"
                );
            }
            hub.wait_for(|value| selected(value)).await?;
        }
        let (gguf, model) = (gguf.clone(), model.clone());
        let client = tokio::task::spawn_blocking(move || -> anyhow::Result<DeciderClient> {
            let checkpoint = match &gguf {
                Some(path) => download::local(&model, path)?,
                None => {
                    tracing::info!(model, "fetching the decider GGUF from the Hugging Face Hub");
                    download::fetch(&model)?
                }
            };
            tracing::info!(
                model = checkpoint.model,
                revision = checkpoint.revision,
                "loading the decider GGUF"
            );
            let client = DeciderClient::load(&checkpoint, options)?;
            tracing::info!(device = client.device(), "selected inference device");
            Ok(client)
        })
        .await??;
        let functions = register(&iii, config.clone(), client);
        tracing::info!("decider model loaded; judge-decider functions registered");
        let Some(hub) = &mut hub else {
            return Ok(());
        };
        hub.wait_for(|value| !selected(value)).await?;
        for function in functions {
            function.unregister();
        }
        tracing::info!("the judge hub no longer selects this provider; decider model released");
    }
}

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tracing::info!("judge-decider ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("judge-decider ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
