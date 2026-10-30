//! memory-consolidate worker: boot, configuration, scheduling.
//!
//! Scheduling REUSES the cron worker: `memory-consolidate::on-tick`
//! observes `cron::on-tick` with an hourly expression, and the tick
//! decides from the persisted last run (state worker, scope
//! `memory_consolidate`) whether `interval_hours` have elapsed. A slim
//! backstop loop provides CATCH-UP-ON-BOOT — a pass missed while this
//! worker was down runs shortly after boot — and keeps the schedule alive
//! on rigs without the cron worker.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, Deliver, IIIClient, InitOptions, RegisterFunction, Source};
use serde_json::json;
use tokio::sync::RwLock;

use memory_consolidate::functions::{self, ConfigCell, Deps};
use memory_consolidate::{config, configuration, manifest, rest};

#[derive(Parser, Debug)]
#[command(
    name = "memory-consolidate",
    about = "Scheduled hygiene for the memory worker: deterministic dedup, supersede-only, pinned untouchable, catch-up-on-boot."
)]
struct Cli {
    /// Engine WebSocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Optional YAML seed config (only seeds the FIRST configuration
    /// registration; the authoritative value lives in the configuration
    /// worker).
    #[arg(long)]
    config: Option<String>,

    /// Print the registry module manifest and exit.
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "memory-consolidate".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// Backstop cadence: catch-up-on-boot plus a safety net on rigs without
/// the cron worker. The tick's due-check makes each pass idempotent, so
/// heartbeat and backstop can coexist.
const BACKSTOP_CHECK: Duration = Duration::from_secs(1_800);
/// First check after boot: give the memory worker and siblings a moment to
/// finish their own boot wave.
const BOOT_GRACE: Duration = Duration::from_secs(45);
/// The cron worker's tick function.
const CRON_ON_TICK_FN: &str = "cron::on-tick";
/// Hourly heartbeat on the cron worker; the tick decides whether a pass is
/// actually due.
const CRON_EXPRESSION: &str = "0 0 * * * *";
/// How long the heartbeat observer may stay inactive before the worker logs
/// that the backstop loop carries the schedule.
const HEARTBEAT_READY_WAIT: Duration = Duration::from_secs(300);

/// What the heartbeat observes: the hourly `cron::on-tick` calls, the tick
/// event delivered as the target's input.
fn heartbeat_source() -> Source {
    Source::function(CRON_ON_TICK_FN)
        .args(json!({ "expression": CRON_EXPRESSION }))
        .deliver(Deliver::Output)
}

/// Register the internal tick handler and observe `cron::on-tick` with it.
/// The observer stays `pending` until the cron worker registers the
/// function (it may boot after this worker), and lives as long as the
/// worker's connection.
fn observe_schedule(iii: &IIIClient, deps: Arc<Deps>) {
    let deps_for_fn = deps.clone();
    iii.register_function(
        functions::TICK_ID,
        RegisterFunction::new_async(move |_tick: functions::TickInput| {
            let deps = deps_for_fn.clone();
            async move { functions::tick(&deps).await }
        })
        .description(functions::TICK_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let heartbeat = iii.observe(heartbeat_source(), functions::TICK_ID);
    tokio::spawn(async move {
        match tokio::time::timeout(HEARTBEAT_READY_WAIT, heartbeat.ready()).await {
            Ok(Ok(())) => tracing::info!(expression = CRON_EXPRESSION, "cron heartbeat active"),
            Ok(Err(e)) => tracing::warn!(
                error = %e,
                "cron heartbeat observer refused; the backstop loop keeps the schedule"
            ),
            Err(_) => tracing::warn!(
                state = %heartbeat.state(),
                "cron heartbeat observer not active yet; the backstop loop keeps the schedule"
            ),
        }
    });
}

/// Catch-up-on-boot plus the no-cron safety net: call the same guarded
/// tick the heartbeat uses.
fn spawn_backstop(deps: Arc<Deps>) {
    tokio::spawn(async move {
        tokio::time::sleep(BOOT_GRACE).await;
        loop {
            if let Err(e) = functions::tick(&deps).await {
                tracing::warn!(error = %e, "scheduled pass failed; retrying next check");
            }
            tokio::time::sleep(BACKSTOP_CHECK).await;
        }
    });
}

/// The REST route of a public function: `POST /memory-consolidate/...`, the
/// function id with `::` as `/`. Its target is `memory-consolidate::rest`.
fn rest_route(function_id: &str) -> Source {
    Source::function("http::on-request").args(json!({
        "method": "POST",
        "path": rest::route_path(function_id),
    }))
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

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let seed = match cli.config.as_deref() {
        Some(path) => match config::WorkerConfig::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load seed config; relying on the stored configuration entry");
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering memory-consolidate configuration schema")?;
    // Convention for worker binaries: a config-load failure degrades to
    // an INERT registration instead of crashing — the worker comes up
    // with the schedule disabled (manual runs still work), and hot
    // reload repairs it once the real configuration is readable. A
    // worker that could not read its config must not start rewriting
    // memory history on default settings.
    let cfg = match configuration::fetch_config(&iii).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load configuration; registering inert (schedule disabled)");
            config::WorkerConfig {
                enabled: false,
                ..config::WorkerConfig::default()
            }
        }
    };

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        config: cell.clone(),
        run_gate: tokio::sync::Mutex::new(()),
    });

    functions::register_all(&iii, &deps);
    rest::register(&iii, &deps);
    configuration::observe_config_changes(&iii, cell);

    // Reuse the http worker: the public functions are also REST routes
    // (`POST /memory-consolidate/...`) served by `memory-consolidate::rest`,
    // which takes the HttpRequest and answers { status_code, body }. They
    // stay pending until the http worker registers `http::on-request`.
    for spec in functions::catalog() {
        let route = iii.observe(rest_route(spec.function_id), rest::REST_ID);
        tracing::debug!(
            function_id = spec.function_id,
            observer_id = route.id(),
            "REST route observed"
        );
    }

    observe_schedule(&iii, deps.clone());
    spawn_backstop(deps);

    tracing::info!(
        "memory-consolidate ready: run/status + cron-heartbeat dedup with catch-up-on-boot"
    );
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heartbeat_observes_the_hourly_tick() {
        assert_eq!(
            heartbeat_source(),
            Source::function("cron::on-tick")
                .args(json!({ "expression": "0 0 * * * *" }))
                .deliver(Deliver::Output)
        );
    }

    #[test]
    fn rest_routes_take_the_leading_slash() {
        assert_eq!(
            rest_route("memory-consolidate::run"),
            Source::function("http::on-request")
                .args(json!({ "method": "POST", "path": "/memory-consolidate/run" }))
        );
    }
}
