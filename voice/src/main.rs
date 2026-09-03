//! The voice worker: speech in, text out, and text back to speech.
//!
//! Boot order, and why:
//!
//! 1. tracing, then the CLI
//! 2. `--manifest` prints and returns without connecting, because the registry
//!    publish pipeline calls it and must not need an engine
//! 3. connect
//! 4. register and fetch the configuration — a required boot dependency
//! 5. register the trigger types, then build the engine, sessions and
//!    speaker, then register the functions and the console UI they drive
//! 6. bind the configuration trigger LAST, so its handler closes over fully
//!    built state; a config change drops the loaded recognizer so the next
//!    call reloads it with the new model or endpointing
//! 7. wait for a signal, close every session so subscribers see the end,
//!    then shut the SDK down cleanly

use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use voice::config::WorkerConfig;
use voice::configuration::ConfigCell;
use voice::engine::{Engine, LoadKey};
use voice::events::{self, Emitter, IiiDeliverer};
use voice::functions::{self, AppState};
use voice::session::Sessions;
use voice::tts::Speaker;
use voice::{configuration, manifest, ui};

#[derive(Parser, Debug)]
#[command(name = "voice", about = manifest::DESCRIPTION)]
struct Cli {
    /// Optional one-time seed for the configuration entry on first
    /// registration. Never overwrites a stored value.
    #[arg(long)]
    config: Option<String>,

    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Print the registry manifest and exit.
    #[arg(long)]
    manifest: bool,
}

/// Wait for either interrupt or terminate: a managed worker is stopped with
/// SIGTERM, and a process that only listens for ctrl-c dies without running
/// `shutdown_async`.
#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())?
        );
        return Ok(());
    }

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "voice".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match WorkerConfig::from_file(path) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to parse config seed; ignoring it");
                None
            }
        });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("configuration::register failed: {e}"))?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(|e| anyhow::anyhow!("configuration::get failed: {e}"))?;
    tracing::info!(
        stt_backend = ?cfg.stt.backend,
        model = %cfg.stt.model,
        tts_backend = ?cfg.tts.backend,
        models_dir = %cfg.models_path().display(),
        "configuration loaded"
    );
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    let sets = events::register_trigger_types(&iii);
    let emitter = Arc::new(Emitter::new(sets, Arc::new(IiiDeliverer::new(iii.clone()))));
    let engine = Arc::new(Engine::new());
    let sessions = Arc::new(Sessions::new(engine.clone(), emitter.clone(), cell.clone()));
    let speaker = Arc::new(Speaker::new());
    let state = Arc::new(AppState {
        cfg: cell.clone(),
        engine: engine.clone(),
        sessions: sessions.clone(),
        speaker: speaker.clone(),
        emitter,
    });

    functions::register_all(&iii, &state);
    ui::register(&iii);

    sessions
        .set_progress_sink(functions::catalog::progress_sink(&state))
        .await;
    let sweep = sessions.spawn_sweep();

    let reload_engine = engine.clone();
    configuration::register_config_trigger(
        &iii,
        cell.clone(),
        Arc::new(move |cfg| {
            let engine = reload_engine.clone();
            tokio::spawn(async move {
                if let Some(loaded) = engine.current().await {
                    if loaded.key != LoadKey::from_config(&cfg) {
                        tracing::info!(
                            "speech model settings changed; the recognizer reloads on next use"
                        );
                        engine.invalidate().await;
                    }
                }
            });
        }),
    )
    .map_err(|e| anyhow::anyhow!("configuration trigger registration failed: {e}"))?;

    tracing::info!(url = %cli.url, "voice worker ready");

    wait_for_shutdown().await?;
    sweep.abort();
    sessions.stop_all().await;
    speaker.stop(None).await;
    iii.shutdown_async().await;
    Ok(())
}
