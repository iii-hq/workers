use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, Deliver, IIIClient, InitOptions, ObserverHandle, Source};
use serde_json::json;

use eval::events::EvalEvents;
use eval::runtime::{self, Deps, TURN_COMPLETED_FN};
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
    // The observers live as long as the worker's connection.
    let _observers = observe(&iii, &deps);

    tracing::info!("eval ready");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

/// The cron function that runs the sweep, and the sweep's schedule.
const ON_TICK_FN: &str = "cron::on-tick";
const SWEEP_EXPRESSION: &str = "*/15 * * * * *";

/// Observe the harness's finished turns (admission and wake-up) and a
/// fifteen-second cron tick (recovery, deadlines and retention), each target
/// taking the event. An observer stays `pending` until its function
/// registers; `eval::config` reports the turn observer's state.
fn observe(iii: &IIIClient, deps: &Deps) -> [ObserverHandle; 2] {
    let turns = iii.observe(
        Source::function(TURN_COMPLETED_FN).deliver(Deliver::Output),
        functions::WAKE_ID,
    );
    deps.observer.set(turns.clone());
    let sweep = iii.observe(
        Source::function(ON_TICK_FN)
            .args(json!({ "expression": SWEEP_EXPRESSION }))
            .deliver(Deliver::Output),
        functions::SWEEP_ID,
    );
    [turns, sweep]
}
