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
//! Write paths are `directory::skills::download*` (bulk materialization)
//! and `directory::skills::update` / `directory::prompts::update`
//! (single-file edits). Read-side surfaces (`directory::skills::list`,
//! `directory::skills::get`, `directory::prompts::*`,
//! `directory::registry::*`) source from the configured `skills_folder`
//! on disk or proxy to the public registry over HTTP. Engine introspection is handled by the engine natively —
//! call `engine::functions::list`, `engine::triggers::list`, etc.,
//! directly.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::json;

use iii_directory::config::{SharedConfig, SkillsConfig};
use iii_directory::functions::download::{
    download_worker_skills, reconcile_decision, InFlightGuard,
};
use iii_directory::functions::registry::RegistryCache;
use iii_directory::functions::skills::{
    make_registered_cache, RegisteredWorkersCache, ENGINE_NAMESPACE,
};
use iii_directory::sources::registry::VersionSpec;
use iii_directory::{configuration, functions, manifest, trigger_types};

#[derive(Parser, Debug)]
#[command(
    name = "iii-directory",
    about = "Engine introspection (functions / triggers / workers), workers registry proxy, and filesystem-backed skill + prompt reader."
)]
struct Cli {
    /// Optional YAML seed used to populate `initial_value` on the first
    /// `configuration::register`. After first boot the authoritative config
    /// lives in the `configuration` worker under id `iii-directory`.
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

    // Connect to the engine first so the configuration RPCs are reachable.
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

    // Optional YAML seed used only to populate `initial_value` on the first
    // `configuration::register`.
    let seed = match cli.config.as_deref() {
        Some(path) => match SkillsConfig::from_file(path) {
            Ok(cfg) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on existing configuration entry"
                );
                None
            }
        },
        None => None,
    };

    // Register the schema (+ seed) and fetch the authoritative, env-expanded
    // value from the `configuration` worker.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering iii-directory configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading iii-directory configuration")?;

    tracing::info!(
        skills_folder = %cfg.resolved_skills_folder().display(),
        local_skills_folder = %cfg.local_skills_folder().display(),
        registry_url = %cfg.registry_base(),
        filter_unregistered = cfg.filter_unregistered,
        auto_download = cfg.auto_download,
        "loaded configuration from the configuration worker"
    );

    // Shared, hot-reloadable state. Topology is captured at boot; tunable
    // fields live in `cfg_handle` (swapped on reload) and the shared
    // `cache_ttl_ms` cell read by both caches.
    let boot_topology = cfg.topology();
    let auto_download = cfg.auto_download;
    let cache_ttl_ms = Arc::new(AtomicU64::new(cfg.registry_cache_ttl_ms));
    let registered_cache = make_registered_cache(cache_ttl_ms.clone());
    let registry_cache = RegistryCache::new_shared(cache_ttl_ms.clone());
    let cfg_handle: SharedConfig = cfg.into_shared();

    // Custom trigger types come first because the download function captures
    // the subscriber sets it'll fan out to on success.
    let registered = trigger_types::register_all(&iii);
    functions::register_all_with_cache(
        &iii,
        &cfg_handle,
        &registered,
        &registered_cache,
        registry_cache.clone(),
    );
    functions::log_fs_health(&cfg_handle.load_full());

    // Injectable console UI: the skills & prompts editor page, the
    // directory::* function-trigger renderer, and the configuration form.
    iii_directory::ui::register(&iii);

    // Auto-download: subscribe to worker add events + boot reconcile. Wired
    // from the boot value of `auto_download` (a topology field — changing it
    // requires a restart).
    if auto_download {
        let in_flight = Arc::new(InFlightGuard::new());
        setup_auto_download(&iii, &cfg_handle, &registered_cache, &in_flight);
        spawn_boot_reconcile(
            iii.clone(),
            cfg_handle.clone(),
            registered_cache.clone(),
            in_flight,
        );
    }

    // Bind the configuration-change trigger so tunable fields hot-reload.
    let state = configuration::SharedState::new(
        cfg_handle.clone(),
        cache_ttl_ms,
        registry_cache,
        registered_cache,
        boot_topology,
    );
    configuration::register_config_trigger(&iii, state)
        .context("registering configuration change trigger")?;

    let fn_count = if auto_download { 14 } else { 13 };
    tracing::info!(
        "iii-directory ready: {} directory::* functions + 2 custom trigger types + \
         configuration hot-reload",
        fn_count
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-directory shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// `worker` trigger payload for `directory::__on_worker_added`. Only `worker`
/// is read; declared as a struct so the function publishes a typed schema.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct WorkerAddedEvent {
    #[serde(default)]
    worker: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct WorkerAddedAck {
    ok: bool,
}

/// Register the internal `directory::__on_worker_added` handler and
/// subscribe to the `worker` trigger type for `add` operations.
fn setup_auto_download(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
    in_flight: &Arc<InFlightGuard>,
) {
    let cfg_inner = cfg.clone();
    let cache_inner = cache.clone();
    let in_flight_inner = in_flight.clone();

    // Register the internal handler that fires on worker-add events.
    iii.register_function(
        "directory::__on_worker_added",
        RegisterFunction::new_async(move |event: WorkerAddedEvent| {
            let cfg = cfg_inner.load_full();
            let cache = cache_inner.clone();
            let in_flight = in_flight_inner.clone();
            async move {
                handle_worker_added(&cfg, &cache, &in_flight, &event).await;
                Ok::<_, Error>(WorkerAddedAck { ok: true })
            }
        })
        .description("Internal: auto-download skills on worker add event.")
        .metadata(serde_json::json!({ "internal": true })),
    );

    // Subscribe to the `worker` trigger type with a retry backoff.
    let iii_sub = iii.clone();
    tokio::spawn(async move {
        for attempt in 1..=5 {
            let result = iii_sub.register_trigger(RegisterTriggerInput {
                trigger_type: "worker".to_string(),
                function_id: "directory::__on_worker_added".to_string(),
                config: json!({
                    "operations": ["add"],
                    "stages": ["done"]
                }),
                metadata: None,
                namespace: iii_sub.namespace(),
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
    event: &WorkerAddedEvent,
) {
    let worker = match event.worker.as_deref() {
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
async fn fetch_worker_list_with_retry(iii: &IIIClient) -> Option<Vec<serde_json::Value>> {
    const MAX_ATTEMPTS: u32 = 6;
    for attempt in 1..=MAX_ATTEMPTS {
        let request = TriggerRequest {
            function_id: "worker::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(10_000),
        };
        let result = match iii.namespace() {
            Some(ns) => iii.trigger(request.namespace(ns)).await,
            None => iii.trigger(request).await,
        };

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
    iii: Arc<IIIClient>,
    cfg: SharedConfig,
    cache: Arc<RegisteredWorkersCache>,
    in_flight: Arc<InFlightGuard>,
) {
    tokio::spawn(async move {
        // Snapshot the live config for this one-shot boot pass. The fields it
        // reads (skills_folder / local_skills_folder) are topology — fixed for
        // the process lifetime — so a snapshot is sufficient.
        let cfg = cfg.load_full();

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
