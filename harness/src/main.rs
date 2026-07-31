//! `harness` binary entry.
//!
//! Boot sequence (configuration.md §4c — Tier 1):
//!   1. Parse CLI. An optional `--config` YAML file only SEEDS the first
//!      registration; the authoritative config lives in the `configuration`
//!      worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative,
//!      env-expanded value (a required boot dependency).
//!   4. Register the trigger types the harness emits/owns (readiness + turn
//!      events + the five hook points) BEFORE functions so handlers capture
//!      subscriber sets.
//!   5. Seed the function-registry cache and bind `engine::functions-available`.
//!   6. Register the `harness::*` functions. Registering `harness::turn`
//!      allows restored queue deliveries to run, but is not external readiness.
//!   7. Ensure the dedicated `harness-turn` queue exists (required; bounded
//!      retries cover queue-worker registration during stack startup).
//!   8. Bind the cron pending-sweep (retain its handle for live re-bind).
//!   9. LAST: bind the configuration-change trigger so its handler closes over
//!      the fully-built snapshot cell + the cron handle.
//!  10. Emit `harness::ready`, then sleep on Ctrl+C and shut down cleanly.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use harness::configuration::{self, ConfigCell, TriggerHandles};
use harness::deps::Deps;
use harness::events::TurnEvents;
use harness::hooks::HookRegistry;
use harness::{config, discovery, functions, manifest, queue, subscriptions};

const BOOT_CONFIG_ATTEMPTS: u32 = 8;
const BOOT_CONFIG_RETRY_BACKOFF_MS: u64 = 500;

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

/// Configuration is the first external boot dependency. A worker-manager
/// restart can bring the harness up before `configuration` has registered its
/// functions, so a single failed RPC must not turn a transient startup race
/// into a permanently dead harness process. Registration is idempotent and
/// the stored value remains authoritative, making this safe across retries.
async fn load_config_with_retries(
    iii: &iii_sdk::IIIClient,
    seed: Option<&config::WorkerConfig>,
) -> Result<config::WorkerConfig> {
    retry_boot_dependency(
        BOOT_CONFIG_ATTEMPTS,
        Duration::from_millis(BOOT_CONFIG_RETRY_BACKOFF_MS),
        || async {
            configuration::register_config(iii, seed).await?;
            configuration::fetch_config(iii).await
        },
    )
    .await
    .map_err(|last_error| {
        anyhow::Error::msg(format!(
            "harness boot dependency initialization failed after {BOOT_CONFIG_ATTEMPTS} attempts: {last_error}"
        ))
    })
}

async fn retry_boot_dependency<T, F, Fut>(
    max_attempts: u32,
    backoff: Duration,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    if max_attempts == 0 {
        return Err("retry attempt count must be greater than zero".to_string());
    }

    let mut last_error = String::new();

    for attempt in 1..=max_attempts {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error;
                if attempt < max_attempts {
                    tracing::warn!(
                        attempt,
                        max_attempts,
                        error = %last_error,
                        "harness boot dependency unavailable; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    Err(last_error)
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

    let cfg = load_config_with_retries(&iii, seed.as_ref())
        .await
        .context("loading harness configuration")?;

    // Trigger types first: the handlers capture the subscriber sets.
    let events = TurnEvents::register(&iii);
    let hooks = HookRegistry::register(&iii);

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let functions_cell = discovery::new_cell();
    let deps = Arc::new(Deps::new(
        iii.clone(),
        cell.clone(),
        functions_cell.clone(),
        events,
        hooks,
    ));

    // The queue worker may already have restored the harness-turn consumer
    // from durable configuration. Seed discovery before registering
    // harness::turn, because target registration is what releases any held
    // delivery waiting for this worker to come back.
    discovery::seed(&iii, &functions_cell, cfg.dispatch_timeout_ms).await;
    discovery::register_functions_trigger(&iii, functions_cell.clone(), cfg.dispatch_timeout_ms);

    functions::register_all(&iii, &deps);

    // The queue consumer may restore durable jobs as soon as this definition
    // succeeds. Discovery and `harness::turn` are already ready; do not bind
    // lifecycle triggers or announce ready unless the queue is usable.
    queue::ensure_turn_queue(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("ensuring the harness-turn queue")?;

    // Bind lifecycle triggers and retain their handles for the worker lifetime.
    // The sweep binding is hot-reloaded when sweep_expression changes.
    let handles = Arc::new(TriggerHandles::new(
        configuration::bind_sweep(&iii, &cfg),
        configuration::bind_session_deleted(&iii),
    ));

    // LAST: bind the configuration-change trigger.
    configuration::register_config_trigger(&iii, cell, handles)
        .context("registering the configuration change trigger")?;

    deps.events.emit_ready().await;
    tracing::info!(
        "harness ready: harness::* functions + subscriptions + turn events + hook points + reactive function-registry cache"
    );

    // Background GC of durable react bindings orphaned across restarts (their
    // in-memory session tracking is gone; their owner session may have been
    // deleted while this harness was down). Non-blocking — never delays ready.
    {
        let deps = deps.clone();
        tokio::spawn(async move { subscriptions::reconcile::run(&deps).await });
    }

    tokio::signal::ctrl_c().await?;
    tracing::info!("harness shutting down");
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test]
    async fn boot_dependency_retries_transient_failures() {
        let attempts = Arc::new(AtomicU32::new(0));

        let result = retry_boot_dependency(4, Duration::ZERO, || {
            let attempts = attempts.clone();
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(format!("not ready on attempt {attempt}"))
                } else {
                    Ok("ready")
                }
            }
        })
        .await;

        assert_eq!(result, Ok("ready"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn boot_dependency_returns_last_error_after_attempt_budget() {
        let attempts = Arc::new(AtomicU32::new(0));

        let result = retry_boot_dependency(3, Duration::ZERO, || {
            let attempts = attempts.clone();
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                Err::<(), _>(format!("failure {attempt}"))
            }
        })
        .await;

        assert_eq!(result, Err("failure 3".to_string()));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn boot_dependency_rejects_an_empty_attempt_budget() {
        let result = retry_boot_dependency(0, Duration::ZERO, || async {
            Ok::<_, String>("must not run")
        })
        .await;

        assert_eq!(
            result,
            Err("retry attempt count must be greater than zero".to_string())
        );
    }
}
