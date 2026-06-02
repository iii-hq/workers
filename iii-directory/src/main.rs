//! `iii-directory` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket.
//!   3. Register the custom trigger types
//!      `directory::skills::on-change` /
//!      `directory::prompts::on-change` (fan-out targets for
//!      `directory::skills::download`).
//!   4. Register every public function against the engine — every
//!      registration sits under `directory::*` (skills, prompts,
//!      registry HTTP proxy).
//!   5. (Optional) Subscribe to `worker` trigger for auto-download on
//!      worker add events and run a boot reconcile for missing skills.
//!   6. Sleep on Ctrl+C, then `shutdown_async` cleanly.
//!
//! `directory::skills::download` is the only write path. Read-side
//! surfaces (`directory::skills::list`, `directory::skills::get`,
//! `directory::prompts::*`, `directory::registry::*`) source from the
//! configured `skills_folder` on disk or proxy to the public registry
//! over HTTP. Engine introspection is handled by the engine natively —
//! call `engine::functions::list`, `engine::triggers::list`, etc.,
//! directly.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, RegisterFunction, TriggerRequest, WorkerMetadata, III,
};
use serde_json::json;

use iii_directory::config::SkillsConfig;
use iii_directory::functions::download::{
    download_worker_skills, reconcile_decision, InFlightGuard,
};
use iii_directory::functions::skills::{
    make_registered_cache, RegisteredWorkersCache, ENGINE_NAMESPACE,
};
use iii_directory::sources::registry::VersionSpec;
use iii_directory::{config, functions, manifest, trigger_types};

#[derive(Parser, Debug)]
#[command(
    name = "iii-directory",
    about = "Engine introspection (functions / triggers / workers), workers registry proxy, and filesystem-backed skill + prompt reader."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
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

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                skills_folder = %c.resolved_skills_folder().display(),
                local_skills_folder = %c.local_skills_folder().display(),
                registry_url = %c.registry_base(),
                filter_unregistered = c.filter_unregistered,
                auto_download = c.auto_download,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SkillsConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "iii-directory".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // Shared registered-workers cache used by read functions and
    // invalidated by the worker-add event handler.
    let cache = make_registered_cache(&cfg);

    // Custom trigger types come first because the download function
    // captures the subscriber sets it'll fan out to on success.
    let registered = trigger_types::register_all(&iii);
    functions::register_all_with_cache(&iii, &cfg, &registered, &cache);
    functions::log_fs_health(&cfg);

    // Auto-download: subscribe to worker add events + boot reconcile.
    if cfg.auto_download {
        let in_flight = Arc::new(InFlightGuard::new());
        setup_auto_download(&iii, &cfg, &cache, &in_flight);
        spawn_boot_reconcile(iii.clone(), cfg.clone(), cache.clone(), in_flight);
    }

    let fn_count = if cfg.auto_download { 10 } else { 9 };
    tracing::info!(
        "iii-directory ready: {} directory::* functions + 2 custom trigger types",
        fn_count
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-directory shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// Register the internal `directory::__on_worker_added` handler and
/// subscribe to the `worker` trigger type for `add` operations.
fn setup_auto_download(
    iii: &Arc<III>,
    cfg: &Arc<SkillsConfig>,
    cache: &Arc<RegisteredWorkersCache>,
    in_flight: &Arc<InFlightGuard>,
) {
    let cfg_inner = cfg.clone();
    let cache_inner = cache.clone();
    let in_flight_inner = in_flight.clone();

    // Register the internal handler that fires on worker-add events.
    iii.register_function(
        "directory::__on_worker_added",
        RegisterFunction::new_async(move |input: serde_json::Value| {
            let cfg = cfg_inner.clone();
            let cache = cache_inner.clone();
            let in_flight = in_flight_inner.clone();
            async move {
                handle_worker_added(&cfg, &cache, &in_flight, &input).await;
                Ok::<_, iii_sdk::IIIError>(json!({"ok": true}))
            }
        })
        .description("Internal: auto-download skills on worker add event."),
    );

    // Subscribe to the `worker` trigger type with a retry backoff.
    let iii_sub = iii.clone();
    tokio::spawn(async move {
        for attempt in 1..=5 {
            let result = iii_sub.register_trigger(iii_sdk::RegisterTriggerInput {
                trigger_type: "worker".to_string(),
                function_id: "directory::__on_worker_added".to_string(),
                config: json!({
                    "operations": ["add"],
                    "stages": ["done"]
                }),
                metadata: None,
            });
            match result {
                Ok(_) => {
                    tracing::info!("subscribed to worker trigger for auto-download");
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        error = %e,
                        "failed to subscribe to worker trigger; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(attempt * 2)).await;
                }
            }
        }
        tracing::warn!(
            "exhausted retries subscribing to worker trigger; \
             auto-download on worker add will not work"
        );
    });
}

/// Handle a `worker` trigger add event. Downloads skills for the
/// newly added worker if not already in-flight.
async fn handle_worker_added(
    cfg: &SkillsConfig,
    cache: &RegisteredWorkersCache,
    in_flight: &Arc<InFlightGuard>,
    payload: &serde_json::Value,
) {
    let worker = match payload.get("worker").and_then(|w| w.as_str()) {
        Some(w) => w.to_string(),
        None => {
            tracing::debug!("worker add event missing 'worker' field; skipping");
            return;
        }
    };

    // RAII: _claim drops at scope end (including on panic/early-return).
    let Some(_claim) = in_flight.claim(&worker) else {
        tracing::debug!(worker = %worker, "worker download already in-flight; skipping");
        return;
    };

    let spec = VersionSpec::Tag("latest".to_string());
    match download_worker_skills(cfg, &worker, &spec).await {
        Ok(true) => {
            tracing::info!(worker = %worker, "auto-download complete on worker add");
            cache.invalidate().await;
        }
        Ok(false) => {
            tracing::debug!(worker = %worker, "no skills bundle for worker (404)");
        }
        Err(e) => {
            tracing::warn!(worker = %worker, error = %e, "auto-download failed on worker add");
        }
    }
}

/// Reconcile a single namespace: claim the in-flight slot and download
/// its skills. Returns `true` iff at least one skill file was written.
/// Shared by the engine-skill reconcile and the per-worker loop.
async fn reconcile_one(
    cfg: &SkillsConfig,
    in_flight: &Arc<InFlightGuard>,
    name: &str,
    spec: &VersionSpec,
) -> bool {
    // RAII: _claim drops when this fn returns (or on panic).
    let Some(_claim) = in_flight.claim(name) else {
        return false;
    };
    match download_worker_skills(cfg, name, spec).await {
        Ok(true) => {
            tracing::info!(worker = name, "boot reconcile: downloaded skills");
            true
        }
        Ok(false) => {
            tracing::debug!(worker = name, "boot reconcile: 404 (benign)");
            false
        }
        Err(e) => {
            tracing::warn!(worker = name, error = %e, "boot reconcile: download failed");
            false
        }
    }
}

/// Fetch the installed-worker list, retrying with backoff while the
/// engine's worker-manager — which registers `worker::list` — is still
/// coming up. On a cold engine start the worker-manager registers late,
/// so `worker::list` reports `function_not_found` for the first few
/// seconds; without a retry the boot reconcile would skip every worker.
///
/// Returns the worker array on success, or `None` if `worker::list`
/// never became available within the retry budget (~30s).
async fn fetch_worker_list_with_retry(iii: &III) -> Option<Vec<serde_json::Value>> {
    const MAX_ATTEMPTS: u32 = 6;
    for attempt in 1..=MAX_ATTEMPTS {
        let result = iii
            .trigger(TriggerRequest {
                function_id: "worker::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(10_000),
            })
            .await;

        match result {
            Ok(val) => {
                return Some(
                    val.get("workers")
                        .and_then(|w| w.as_array())
                        .cloned()
                        .unwrap_or_default(),
                );
            }
            Err(e) if attempt == MAX_ATTEMPTS => {
                tracing::warn!(
                    attempt,
                    error = %e,
                    "boot reconcile: worker::list unavailable after retries; skipping worker reconcile"
                );
                return None;
            }
            Err(e) => {
                tracing::debug!(
                    attempt,
                    error = %e,
                    "boot reconcile: worker::list not ready (worker-manager still coming up); retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(attempt) * 2)).await;
            }
        }
    }
    None
}

/// Spawn a non-blocking boot reconcile task. Always ensures the engine's
/// own skill (`iii`) is present, then fetches the installed worker list
/// and downloads skills for any worker whose global namespace is
/// absent/incomplete (no completion marker) AND has no local override
/// AND name validates.
fn spawn_boot_reconcile(
    iii: Arc<III>,
    cfg: Arc<SkillsConfig>,
    cache: Arc<RegisteredWorkersCache>,
    in_flight: Arc<InFlightGuard>,
) {
    tokio::spawn(async move {
        // Small delay so the engine has time to wire us up.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let global_root = cfg.resolved_skills_folder();
        let local_root = cfg.local_skills_folder();
        let mut reconciled = 0u32;

        // Always ensure the engine's own skill is present. The engine is
        // not a worker, so it never appears in `worker::list`; reconcile
        // it directly (registry pull), independent of — and before — the
        // worker list, so it lands even when `worker::list` isn't ready
        // yet on a cold start.
        if let Some(spec) = reconcile_decision(ENGINE_NAMESPACE, None, &local_root, &global_root) {
            if reconcile_one(&cfg, &in_flight, ENGINE_NAMESPACE, &spec).await {
                reconciled += 1;
            }
        }

        // Retry `worker::list` with backoff: the worker-manager that
        // provides it registers late on a cold engine start. The engine
        // skill was already reconciled above, independent of this call.
        let workers = match fetch_worker_list_with_retry(&iii).await {
            Some(w) => w,
            None => {
                if reconciled > 0 {
                    cache.invalidate().await;
                }
                tracing::info!(
                    workers = 0,
                    reconciled,
                    "boot reconcile complete (engine skill only)"
                );
                return;
            }
        };

        for w in &workers {
            let name = match w.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let version = w.get("version").and_then(|v| v.as_str());
            let spec = match reconcile_decision(name, version, &local_root, &global_root) {
                Some(s) => s,
                None => continue,
            };

            if reconcile_one(&cfg, &in_flight, name, &spec).await {
                reconciled += 1;
            }
        }

        if reconciled > 0 {
            cache.invalidate().await;
        }

        tracing::info!(
            workers = workers.len(),
            reconciled,
            "boot reconcile complete"
        );
    });
}
