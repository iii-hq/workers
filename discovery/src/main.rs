//! `discovery` binary entry: connect, snapshot the engine catalog, register
//! the search function + hook, bind the catalog push and the pre-generate
//! hook, wire the configuration entry, then sleep until Ctrl+C.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use discovery::{config, functions, ui};

#[derive(Parser, Debug)]
#[command(
    name = "discovery",
    about = "One-shot lexical function search for agents on the iii bus."
)]
struct Cli {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
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
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "discovery".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "One-shot lexical function search for agents on the iii bus.".to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let catalog: functions::CatalogCell = Arc::new(RwLock::new(Arc::new(Vec::new())));
    if functions::refresh_catalog(&iii, &catalog).await.is_err() {
        tracing::warn!("catalog refresh failed; starting with an empty catalog");
    }

    let config_cell: config::ConfigCell = Arc::new(RwLock::new(config::DiscoveryConfig::default()));
    let deps = functions::Deps {
        config: config_cell.clone(),
        catalog: catalog.clone(),
        sessions: Arc::default(),
        iii: Some(iii.clone()),
    };
    functions::register_all(&iii, &deps);
    functions::bind_best_effort(&iii);
    ui::register(&iii);
    let hint_binding = discovery::hook::HintBindingState::default();

    // Best-effort on purpose: the entry carries one prompt-shaping knob, so a
    // configuration-worker failure must not take search_functions off the
    // bus — warn, run on defaults, recover on the next change or restart.
    if let Err(e) = config::register_config(&iii).await {
        tracing::warn!(error = %e, "registering discovery configuration schema failed; continuing");
    }
    match config::fetch_config(&iii).await {
        Ok(cfg) => *config_cell.write().await = cfg,
        Err(e) => {
            tracing::warn!(error = %e, "loading discovery configuration failed; using defaults")
        }
    }
    discovery::hook::apply(&iii, &hint_binding, config_cell.read().await.inject_hint);
    match config::register_config_trigger(&iii, config_cell.clone(), hint_binding.clone()) {
        // One serialized re-fetch to close the boot gap: an update landing
        // between the fetch above and the binding just registered fired into
        // nothing, and would otherwise stay invisible until the NEXT change.
        Ok(reload) => reload.run().await,
        Err(e) => {
            tracing::warn!(error = %e, "registering configuration change trigger failed; hint_min_workers is frozen until restart");
        }
    }

    tracing::info!(
        functions = catalog.read().await.len(),
        hint_min_workers = config_cell.read().await.hint_min_workers,
        "discovery ready"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("discovery shutting down");
    iii.shutdown_async().await;
    Ok(())
}
