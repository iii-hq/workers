use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use compose_ui::watch::{ChangedEvent, ChangedTriggerHandler, ChangedTriggerSpec};
use compose_ui::{functions, ui};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions, RegisterTriggerType};

const WORKER: &str = "compose-ui";
const DESCRIPTION: &str = "Compose project supervision in the Console: live container state, lifecycle actions, worker packages, and log tails.";

#[derive(Debug, Parser)]
#[command(name = WORKER, version, about = DESCRIPTION)]
struct Cli {
    /// WebSocket URL of the iii engine. Also read from III_URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: WORKER.to_string(),
        os: std::env::consts::OS.to_string(),
        description: Some(DESCRIPTION.to_string()),
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
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let changed = ChangedTriggerHandler::new(iii.clone());
    let watcher = changed.watcher();
    let _changed_trigger = iii.register_trigger_type(
        RegisterTriggerType::new(
            "compose-ui::changed",
            "Fires when the compose daemon writes state.json or the compose file changes. Bind with an empty config.",
            changed,
        )
        .trigger_request_format::<ChangedTriggerSpec>()
        .call_request_format::<ChangedEvent>(),
    );

    functions::register(&iii, watcher.clone());
    ui::register(&iii);

    match watcher.ensure().await {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::warn!("compose daemon not reachable yet; watching starts on the next binding")
        }
        Err(error) => tracing::warn!(error = %error, "compose daemon not reachable yet"),
    }

    tracing::info!("compose-ui ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("compose-ui shutting down");
    watcher.close().await;
    iii.shutdown_async().await;
    Ok(())
}
