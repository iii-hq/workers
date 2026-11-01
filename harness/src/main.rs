//! `harness` binary entry.
//!
//! Boot sequence (configuration.md §4c — Tier 1):
//!   1. Parse CLI. An optional `--config` YAML file only SEEDS the first
//!      registration; the authoritative config lives in the `configuration`
//!      worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative,
//!      env-expanded value (a required boot dependency).
//!   4. Register the observable functions the harness hosts (readiness + turn
//!      events + the six hook points), each with its observer hooks set by id
//!      before it registers, so every observer is accepted through them.
//!   5. Seed the function and skill catalog caches and observe their change
//!      events.
//!   6. Register the `harness::*` functions and `harness::on-timer`.
//!      Registering `harness::turn` allows restored queue deliveries to run,
//!      but is not external readiness.
//!   7. Ensure the dedicated `harness-turn` queue exists (required; bounded
//!      retries cover queue-worker registration during stack startup).
//!   8. Observe `cron::on-tick` for the pending-call sweep (retain its handle
//!      for a live replacement) and `session::on-deleted`.
//!   9. LAST: observe `configuration::on-updated` so its handler closes over
//!      the fully-built snapshot cell + the sweep observer.
//!  10. Call `harness::on-ready`, run the boot recovery passes, observe the
//!      engine's worker changes that re-run them, then sleep on Ctrl+C and
//!      shut down cleanly. Nothing in the worker runs on a periodic timer.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use harness::configuration::{self, ConfigCell, LifecycleObservers};
use harness::deps::Deps;
use harness::events::TurnEvents;
use harness::hooks::HookRegistry;
use harness::{config, discovery, functions, manifest, queue};

/// Asset names + embedded bytes for the injected console UI; the registration
/// contract itself lives in the shared `iii-console-ui` crate.
mod ui;

#[derive(Parser, Debug)]
#[command(
    name = "harness",
    about = "Thin durable turn loop wiring session-manager, context-manager, and llm-router."
)]
struct Cli {
    /// Optional seed config.yaml for the first registration only. The
    /// authoritative config is always fetched from the configuration worker.
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

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "harness".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative value comes from the
    // configuration worker).
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
        .context("registering harness configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading harness configuration")?;

    // The harness's own observable functions first: each sets its observer
    // hooks by id before it registers.
    let events = TurnEvents::register(&iii);
    let hooks = HookRegistry::register(&iii);

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let functions_cell = discovery::new_cell();
    let skills_cell = harness::skills::new_cell();
    let deps = Arc::new(Deps::new(
        iii.clone(),
        cell.clone(),
        functions_cell.clone(),
        skills_cell.clone(),
        events,
        hooks,
    ));

    // The queue worker may already have restored the harness-turn consumer
    // from durable configuration. Seed discovery before registering
    // harness::turn, because target registration is what releases any held
    // delivery waiting for this worker to come back.
    discovery::seed(&iii, &functions_cell, cfg.dispatch_timeout_ms).await;
    discovery::observe_function_changes(&iii, functions_cell.clone(), cfg.dispatch_timeout_ms);
    harness::skills::seed(&iii, &skills_cell, cfg.dispatch_timeout_ms).await;
    harness::skills::observe_skill_changes(&iii, skills_cell, cfg.dispatch_timeout_ms);

    functions::register_all(&iii, &deps);

    // Injectable console UI: content function + one `console::ui::asset`
    // observer per asset. Ordering doesn't matter for the console side (an
    // observer stays pending until the console hosts the function), but the
    // content function must exist before an observer names it.
    ui::register(&iii);

    // The one-shot deadline: `harness::on-timer`, whose observers replay to
    // its added hook after a restart (re-armed at the same absolute `at` —
    // the `harness::observe` intercept resolves `in_ms` before observing).
    let _timer = harness::timer::TimerBus::register(&iii);

    // The queue consumer may restore durable jobs as soon as this definition
    // succeeds. Discovery and `harness::turn` are already ready; do not
    // observe lifecycle events or announce ready unless the queue is usable.
    queue::ensure_turn_queue(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("ensuring harness turn and deletion queues before recovery")?;

    // Observe lifecycle events and retain the handles for the worker
    // lifetime. The sweep observer is replaced when sweep_expression changes.
    let handles = Arc::new(LifecycleObservers::new(
        configuration::observe_sweep(&iii, &cfg),
        configuration::observe_session_deleted(&iii),
    ));

    // LAST: observe configuration changes.
    let _config_observer = configuration::observe_config_changes(&iii, cell, handles);

    harness::functions::delete_session_tree::recover(&deps)
        .await
        .context("recovering durable session deletions")?;
    deps.events.emit_ready().await;
    tracing::info!(
        "harness ready: harness::* functions + observers + turn events + hook points + reactive function-registry cache"
    );

    // Background GC of durable bindings orphaned across restarts (their owner
    // session may have been deleted while this harness was down), the replay
    // that re-creates every surviving binding's observer, then the binding
    // pass: it
    // retires lifecycle-spent bindings (an unfired wake past its deadline
    // wakes its owner) and arms every other deadline as its own timer — the
    // timer, not a sweep, is what turns `expires_at` into that notice. The
    // pass re-runs on each engine worker change. Non-blocking — never delays
    // ready.
    {
        let deps = deps.clone();
        tokio::spawn(async move {
            harness::bindings::gc::run(&deps).await;
            harness::bindings::expiry::run(deps).await;
        });
    }
    // Sessions the store still reports `working` after a restart (a terminal
    // projection lost while the session-manager was down): re-derive their
    // status from the turn record. Non-blocking.
    {
        let deps = deps.clone();
        tokio::spawn(async move { harness::session_status::sweep(&deps).await });
    }
    // Orphaned-turn recovery: a `Running` turn whose step never reached the
    // queue (enqueue failed during an outage) is re-enqueued so it resumes
    // or observes its stop instead of wedging the session. One pass after
    // boot, then one per engine worker change plus known-due follow-ups.
    tokio::spawn(harness::inflight::run(deps.clone()));
    // Compose wakes whose operation finished inside their activation window.
    harness::bindings::compose::observe_terminal_operations(&deps);
    // The engine's worker connect/disconnect/announce events: kick the
    // binding pass, the orphaned-turn pass, and (on announce) the catalog
    // refresh.
    harness::engine_events::register(&deps);

    tokio::signal::ctrl_c().await?;
    tracing::info!("harness shutting down");
    iii.shutdown_async().await;
    Ok(())
}
