//! `session-manager` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults), then
//!      resolve the storage backend — an invalid `backend_config` is
//!      **fatal** (a misconfigured bridge must never silently fall back
//!      to a local fs store).
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the six public trigger types (`session::created`, ...)
//!      — first, because the function handlers capture the subscriber
//!      sets they fan out to.
//!   4. Per backend:
//!      - **fs**: open the data_dir, register the internal
//!        `session::store::events` feed + the `session::store::*` raw
//!        protocol, and emit locally (plus envelope fan-out to every
//!        attached bridged instance).
//!      - **bridge**: connect to the main instance, store via its
//!        `session::store::*`, publish events to it
//!        (`session::store::publish_events`), and attach a relay to its
//!        `session::store::events` feed so local subscribers receive
//!        every participant's events.
//!   5. Register the 14 `session::*` functions.
//!   6. Sleep on Ctrl+C, then `shutdown_async` cleanly (both
//!      connections in bridge mode).

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata, III};

use session_manager::config::Backend;
use session_manager::events::{Emitter, EventSink, IiiDeliverer, RemotePublisher};
use session_manager::functions::{store_protocol, Deps};
use session_manager::service::SessionService;
use session_manager::store::{BridgeStore, FsStore, SessionStore};
use session_manager::{config, events, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "session-manager",
    about = "Durable, reactive, branching store of typed conversation entries."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "session-manager".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::WorkerConfig::default()
        }
    };
    let backend = cfg
        .resolve_backend()
        .context("invalid backend configuration — refusing to start")?;
    let cfg = Arc::new(cfg);

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // The six public trigger types come first: the function handlers
    // capture the subscriber sets they fan out to. The engine replays
    // any existing trigger registrations back to us, so the sets
    // self-rebuild after a worker restart.
    let sets = events::register_trigger_types(&iii);

    // `remote` outlives the loop so bridge mode can shut it down too.
    let mut remote_handle: Option<Arc<III>> = None;

    let deps = match backend {
        Backend::Fs(fs_cfg) => {
            let data_dir = fs_cfg.resolved_data_dir();
            let store: Arc<dyn SessionStore> = Arc::new(
                FsStore::new(&data_dir)
                    .with_context(|| format!("open data_dir {}", data_dir.display()))?,
            );

            // Main mode: serve the envelope feed + the raw store
            // protocol for bridged instances.
            let bridges = events::register_store_events_type(&iii);
            let emitter = Arc::new(Emitter::with_bridges(
                sets,
                Arc::new(IiiDeliverer::new(iii.clone())),
                bridges,
            ));
            store_protocol::register_store_protocol(&iii, store.clone(), emitter.clone());

            let service = Arc::new(SessionService::new(store, &cfg));
            let sink: Arc<dyn EventSink> = emitter;
            tracing::info!(data_dir = %data_dir.display(), "backend: fs (main)");
            Arc::new(Deps { service, sink })
        }
        Backend::Bridge(bridge_cfg) => {
            if bridge_cfg.url == cli.url {
                tracing::warn!(
                    url = %bridge_cfg.url,
                    "bridge url equals the local engine url — this instance would defer to \
                     itself; check backend_config.url"
                );
            }
            let remote = Arc::new(register_worker(
                &bridge_cfg.url,
                InitOptions {
                    metadata: Some(worker_metadata()),
                    ..InitOptions::default()
                },
            ));
            remote_handle = Some(remote.clone());

            let store: Arc<dyn SessionStore> =
                Arc::new(BridgeStore::new(remote.clone(), bridge_cfg.timeout_ms));

            // Local emitter serves local subscribers; it is fed by the
            // relay (events coming back from the main), never directly
            // by the handlers.
            let local_emitter =
                Arc::new(Emitter::new(sets, Arc::new(IiiDeliverer::new(iii.clone()))));
            let relay = events::attach_bridge_relay(&remote, local_emitter);

            let service = Arc::new(SessionService::new(store, &cfg));
            let sink: Arc<dyn EventSink> =
                Arc::new(RemotePublisher::new(remote, bridge_cfg.timeout_ms));
            tracing::info!(main = %bridge_cfg.url, relay = %relay, "backend: bridge");
            Arc::new(Deps { service, sink })
        }
    };

    functions::register_all(&iii, &deps);

    tracing::info!("session-manager ready: 14 session::* functions + 6 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("session-manager shutting down");
    if let Some(remote) = remote_handle {
        remote.shutdown_async().await;
    }
    iii.shutdown_async().await;
    Ok(())
}
