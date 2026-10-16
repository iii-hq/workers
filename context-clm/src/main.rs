//! `context-clm` binary: connect, register and read the `context-clm`
//! configuration entry, register the strategy, status and the
//! `harness::turn-completed` and `session::deleted` cleanups, follow
//! configuration changes, serve until SIGINT/SIGTERM.
use std::sync::Arc;

use clap::Parser;
use context_clm::assemble::{
    self, Deps, IiiDelegate, Locks, SessionDeletedEvent, TurnCompletedEvent,
};
use context_clm::configuration;
use context_clm::status::{self, StatusRequest};
use context_clm::store::Store;
use context_clm::types::AssembleRequest;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::json;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "context-clm",
    about = "Model-managed context for the harness: the model edits a mirror of its own context under a token budget."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> anyhow::Result<()> {
    // Handler payloads are whole transcripts; keep them out of span events.
    // Set before the runtime spawns any thread.
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
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: context_clm::WORKER_NAME.into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    // `configuration` is a required boot dependency: a failed register/fetch
    // aborts startup instead of serving on guessed settings.
    configuration::register_config(&iii)
        .await
        .map_err(anyhow::Error::msg)?;
    let config = configuration::new_cell(
        configuration::fetch_config(&iii)
            .await
            .map_err(anyhow::Error::msg)?,
    );
    configuration::register_identity(&iii);
    // NOTE: data_dir is read once at boot; changing it takes a restart.
    let store = Store::new(iii_worker_paths::resolve_path(
        &config.read().await.data_dir,
    ));
    let deps = Arc::new(Deps {
        delegate: IiiDelegate(iii.clone()),
        store,
        config: config.clone(),
        locks: Locks::default(),
    });
    register_functions(&iii, &deps);
    // Held for the process lifetime; config-client logs a failed bind. The
    // binding config stays `{}`: session-manager rejects unknown keys.
    let _session_deleted = iii_config_client::try_bind(
        &iii,
        RegisterTriggerInput::new(
            "session::deleted",
            assemble::ON_SESSION_DELETED_ID,
            json!({}),
        ),
    );
    // `{}`: every session's turns; the handler acts only on CLM state.
    let _turn_completed = iii_config_client::try_bind(
        &iii,
        RegisterTriggerInput::new(
            "harness::turn-completed",
            assemble::ON_TURN_COMPLETED_ID,
            json!({}),
        ),
    );
    // Bound last so the reload closes over fully built state.
    configuration::register_config_trigger(&iii, config)?
        .run()
        .await;
    let result = wait_for_shutdown().await;
    // shutdown() joins the SDK's connection thread so pending telemetry can
    // flush before main returns.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}

fn register_functions(iii: &IIIClient, deps: &Arc<Deps<IiiDelegate>>) {
    let d = deps.clone();
    iii.register_function(
        assemble::ASSEMBLE_ID,
        RegisterFunction::new_async(move |req: AssembleRequest| {
            let d = d.clone();
            async move { assemble::assemble(&d, req).await }
        })
        .description(
            "The context::assemble contract with model-managed context (CLM): the model edits a \
             mirror of its own context under a token budget. Called by the harness for sessions \
             whose metadata sets context_strategy to \"clm\".",
        )
        .metadata(json!({ "internal": true })),
    );
    let d = deps.clone();
    iii.register_function(
        status::STATUS_ID,
        RegisterFunction::new_async(move |req: StatusRequest| {
            let d = d.clone();
            async move { status::status(&d, req).await }
        })
        .description(
            "One CLM session's projection size, effective budget, calibration ratio and \
             counters (steps, edits applied/rejected, rollbacks, stops, peak size, thinking strips).",
        ),
    );
    let d = deps.clone();
    iii.register_function(
        assemble::ON_SESSION_DELETED_ID,
        RegisterFunction::new_async(move |event: SessionDeletedEvent| {
            let d = d.clone();
            async move { assemble::on_session_deleted(&d, event).await }
        })
        .description("Internal: remove a deleted session's CLM mirror, state and step log.")
        .metadata(json!({ "internal": true })),
    );
    let d = deps.clone();
    iii.register_function(
        assemble::ON_TURN_COMPLETED_ID,
        RegisterFunction::new_async(move |event: TurnCompletedEvent| {
            let d = d.clone();
            async move { assemble::on_turn_completed(&d, event).await }
        })
        .description(
            "Internal: remove a finished turn's CLM mirror (and an unused .clm) from the \
             session's root; the next step writes it again.",
        )
        .metadata(json!({ "internal": true })),
    );
}

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tracing::info!("context-clm ready");
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    tracing::info!("context-clm shutting down");
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tracing::info!("context-clm ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("context-clm shutting down");
    Ok(())
}
