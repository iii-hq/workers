use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::trigger::Trigger;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

use eval::events::EvalEvents;
use eval::runtime::{self, Deps};
use eval::{functions, manifest, queue, ui};

#[derive(Parser, Debug)]
#[command(name = "eval", about = "Session monitor for the iii Harness.")]
struct Cli {
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
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "eval".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            otel: Some(OtelConfig::default()),
            ..InitOptions::default()
        },
    ));

    let events = EvalEvents::register(&iii);
    let deps = Deps::new(iii.clone(), events);
    functions::register_all(&iii, &deps);
    ui::register(&iii);
    queue::ensure_run_queue(&iii)
        .await
        .context("ensuring eval-run queue")?;

    // Publish indexes saved before a crash and resume pending analyses before
    // any new observation can be admitted.
    match runtime::recover(&deps).await {
        Ok(recovered) => tracing::info!(?recovered, "eval startup recovery finished"),
        Err(error) => tracing::warn!(%error, "eval startup recovery failed; the sweep retries"),
    }
    let _trigger_handles = bind_triggers(&iii, &deps);

    tracing::info!("eval ready");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

fn bind_triggers(iii: &Arc<iii_sdk::IIIClient>, deps: &Deps) -> Vec<Trigger> {
    let mut handles = Vec::new();
    for (trigger_type, function_id, config) in [
        ("harness::turn-completed", functions::WAKE_ID, json!({})),
        (
            "cron",
            functions::SWEEP_ID,
            json!({ "expression": "*/15 * * * * *" }),
        ),
    ] {
        match iii.register_trigger(RegisterTriggerInput::new(trigger_type, function_id, config)) {
            Ok(handle) => {
                if function_id == functions::WAKE_ID {
                    deps.observer.set(Ok(()));
                }
                handles.push(handle);
            }
            Err(error) => {
                tracing::warn!(
                    trigger_type,
                    function_id,
                    %error,
                    "eval trigger binding failed"
                );
                if function_id == functions::WAKE_ID {
                    // Shown by eval::config: automatic observation is
                    // unavailable even when enabled.
                    deps.observer.set(Err(error.to_string()));
                }
            }
        }
    }
    handles
}
