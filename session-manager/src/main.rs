//! `session-manager` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file is only a SEED for the
//!      first registration; the authoritative config lives in the
//!      `configuration` worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) with the `configuration` worker
//!      and fetch the authoritative, env-expanded value — an invalid bridge
//!      `config` is **fatal** at parse time (a misconfigured bridge must never
//!      silently fall back to a local fs store). `configuration` is a required
//!      boot dependency.
//!   4. Register the six private event functions (`session::on-created`, ...)
//!      and the internal `session::store::on-events` feed.
//!   5. Build the storage + event runtime from the `adapter` config
//!      (`build_runtime`) and wrap it in the shared, hot-swappable `AppState`.
//!   6. Register the `session::store::*` raw protocol (mode-gated: served only
//!      while in fs mode), the 21 `session::*` functions (which read the live
//!      runtime per call), the `session::on-config-change` handler with its
//!      observer of `configuration::on-updated` (which rebuilds and swaps the
//!      runtime on an adapter change), and `session::config-status`.
//!   7. Sleep on Ctrl+C, then `shutdown_async` cleanly (both connections when a
//!      bridge runtime is live).
//!
//! Because the runtime is swappable, an `adapter` change (fs <-> bridge, a new
//! `data_dir`, a bridge url/timeout) hot-reloads without a restart; the new
//! store's state is replayed through the event functions so observers stay
//! live.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use session_manager::configuration::{self, AppState, ConfigCell};
use session_manager::functions::{self, store_protocol};
use session_manager::runtime::{build_runtime, worker_metadata, SessionBuildContext};
use session_manager::{config, events, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "session-manager",
    about = "Durable, reactive, branching store of typed conversation entries."
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on the
    /// first registration. The AUTHORITATIVE config is always fetched from
    /// the `configuration` worker afterward; this file only seeds it.
    #[arg(long)]
    config: Option<String>,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
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

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative config comes from the
    // configuration worker). The seed IS env-expanded (`${VAR}`).
    let seed = match cli.config.as_deref() {
        Some(path) => match config::WorkerConfig::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on the stored configuration entry"
                );
                None
            }
        },
        None => None,
    };

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // Register the schema (+ optional seed) and fetch the authoritative,
    // env-expanded config. `configuration` is a required boot dependency: a
    // failed register/fetch aborts startup.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering session-manager configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading session-manager configuration")?;

    // The shared config snapshot the service reads list limits from per call.
    let config_cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));

    // The six event functions + the internal store-events feed. Both are
    // registered whatever the adapter (even when booting in bridge mode) so a
    // later bridge->fs hot-reload needs no registration. The engine keeps each
    // function's observers across a worker restart and hands them back when
    // the functions register again.
    events::register_event_functions(&iii);

    let ctx = Arc::new(SessionBuildContext {
        iii: iii.clone(),
        local_url: cli.url.clone(),
        config_cell,
    });

    // Build the initial runtime. An invalid bridge `config` (e.g. a self-
    // referential url) is fatal here — a misconfigured bridge never silently
    // falls back to a local fs store.
    let runtime = build_runtime(&cfg, &ctx)
        .map_err(anyhow::Error::msg)
        .context("building the session-manager storage runtime")?;
    let state = AppState::new(runtime, ctx);

    // Raw store protocol (served only while in fs mode), then the public
    // functions (each reads the live runtime per call), then the config-change
    // observer and the config-status surface.
    store_protocol::register_store_protocol(&iii, state.clone());
    functions::register_all(&iii, &state);
    configuration::register_config_observer(&iii, state.clone());
    configuration::register_config_status(&iii, state.clone());

    tracing::info!(
        "session-manager ready: 21 session::* functions + 6 session::on-* event functions (adapter hot-reloadable)"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("session-manager shutting down");
    // Retire the bridge relay and remote connection too, if a bridge runtime
    // is live.
    state.runtime.read().await.close_bridge().await;
    iii.shutdown_async().await;
    Ok(())
}
