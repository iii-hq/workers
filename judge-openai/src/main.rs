//! Boot the OpenAI Decisions provider using the shared configuration-client lifecycle.
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use judge_openai::{configuration, register, DecisionsClient};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "judge-openai",
    about = "OpenAI Decisions provider for the judge hub: typed Noul, Choice and Score evaluations over arbitrary JSON state."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> anyhow::Result<()> {
    // The SDK records every handler's input and output as span events; inputs
    // here are caller content. Set before the runtime spawns any thread.
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
    // Operator opt-in: the name is shared with llm-router, so a worker started
    // with that environment answers with the same key unless api_key is set.
    let client = DecisionsClient::new(std::env::var("OPENAI_API_KEY").ok());
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "judge-openai".into(),
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
    let config = configuration::new_cell(
        configuration::fetch_config(&iii)
            .await
            .map_err(anyhow::Error::msg)?,
    );
    let secrets = register(&iii, config.clone(), client);
    judge_provider::secrets::register_secret_trigger(
        &iii,
        secrets,
        judge_openai::SECRET_CHANGED_ID,
    );
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
    tracing::info!("OpenAI judge worker ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("OpenAI judge worker ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
