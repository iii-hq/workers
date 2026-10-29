//! `iii-directory` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket.
//!   3. Register the change functions
//!      `directory::skills::on-change` /
//!      `directory::system-prompts::on-change` /
//!      `directory::agents::on-change` / `directory::kits::on-change`.
//!   4. Register every public function against the engine — every
//!      registration sits under `directory::*` (search, skills, system
//!      prompts, agents, registry, and one engine-info wrapper).
//!   5. (Optional) Observe `engine::on-workers-changed` for auto-download
//!      when a worker connects, and run a boot reconcile for missing skills.
//!   6. Start the filesystem watch so external edits under the directory
//!      roots call the matching `on-change` (best-effort, never fatal).
//!   7. Sleep on Ctrl+C, then `shutdown_async` cleanly.
//!
//! Write paths are `directory::skills::download*` (bulk materialization)
//! and `directory::skills::{update,create,delete}` /
//! `directory::system-prompts::{update,create,delete}` /
//! `directory::agents::{update,create,delete}` (single-file edits). Read-side
//! surfaces include search, skills, system prompts, agents, and registry.
//! Filesystem content comes from the configured `skills_folder`
//! on disk (plus the read-only `agents_skills_folder` for system-installed
//! agent skills); registry calls proxy the public registry over HTTP. Engine
//! introspection is native, with `directory::engine::functions::info` retained
//! as a narrow wrapper for restricted callers.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};

use iii_directory::config::{FunctionSearchMode, SharedConfig, SkillsConfig};
use iii_directory::functions::download::{
    download_worker_skills, reconcile_decision, InFlightGuard,
};
use iii_directory::functions::registry::RegistryCache;
use iii_directory::functions::skills::{
    compose_status_request, make_registered_cache, parse_worker_names, RegisteredWorkersCache,
    ENGINE_NAMESPACE,
};
use iii_directory::sources::registry::VersionSpec;
use iii_directory::{configuration, events, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "iii-directory",
    about = "Engine introspection, workers registry proxy, filesystem-backed skills, system prompts, and agent profiles, plus lexical function search with a conditional pre-generate hint."
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
        agents_folder = %cfg.resolved_agents_folder().display(),
        registry_url = %cfg.registry_base(),
        filter_unregistered = cfg.filter_unregistered,
        auto_download = cfg.auto_download,
        "loaded configuration from the configuration worker"
    );

    // Shared, hot-reloadable state. Topology is captured at boot; tunable
    // fields live in `cfg_handle` (swapped on reload) and the shared
    // `cache_ttl_ms` cell read by both caches.
    let boot_topology = cfg.topology();
    let function_search_model_path = cfg.resolved_function_search_model_path();
    // MiniLM is compiled only for targets with a pinned ONNX Runtime (see
    // build.rs); elsewhere Hybrid serves BM25 and neither a download
    // nor a missing-bundle warning helps.
    let minilm_supported = cfg!(minilm);
    if !minilm_supported && cfg.function_search_mode != FunctionSearchMode::Lexical {
        tracing::info!(
            mode = ?cfg.function_search_mode,
            target = env!("TARGET"),
            "no pinned ONNX Runtime for this target; this build serves BM25 only"
        );
    }
    // A missing bundle is fetched in the background AFTER the functions are
    // registered (below): ~180 MB must never sit between boot and
    // `directory::*` being callable. BM25 serves until the index is built.
    let bundle_ready = function_search_model_path
        .as_deref()
        .is_some_and(functions::search_semantic::bundle_complete);
    let download_bundle = minilm_supported
        && cfg.function_search_mode != FunctionSearchMode::Lexical
        && cfg.function_search_model_download
        && function_search_model_path.is_some()
        && !bundle_ready;
    if minilm_supported && !download_bundle {
        iii_directory::config::warn_if_search_mode_lacks_model(
            cfg.function_search_mode,
            function_search_model_path.is_some(),
            bundle_ready,
        );
    }
    let auto_download = cfg.auto_download;
    let cache_ttl_ms = Arc::new(AtomicU64::new(cfg.registry_cache_ttl_ms));
    let registered_cache = make_registered_cache(cache_ttl_ms.clone());
    let registry_cache = RegistryCache::new_shared(cache_ttl_ms.clone());
    let cfg_handle: SharedConfig = cfg.into_shared();

    // The change functions come first: the write functions call them on
    // success.
    let sources = events::register_all(&iii);
    functions::register_all_with_cache(
        &iii,
        &cfg_handle,
        &sources,
        &registered_cache,
        registry_cache.clone(),
    );
    functions::log_fs_health(&cfg_handle.load_full());

    // Function search: seed the engine-catalog snapshot (best effort — the
    // engine::on-functions-changed observer refreshes it), register
    // directory::search_functions + its internal hook/preview/listener, and
    // reconcile the pre-generate hint observer with the inject_hint knob.
    let search_catalog: functions::search::CatalogCell =
        Arc::new(tokio::sync::RwLock::new(Arc::new(Vec::new())));
    let semantic =
        functions::search_semantic::SemanticSearch::new(function_search_model_path.clone());
    // Keep an installed model ready for the judge's Hybrid fallback as well.
    semantic.set_enabled(cfg_handle.load().function_search_mode != FunctionSearchMode::Lexical);
    if functions::search::refresh_catalog(&iii, &search_catalog, &semantic)
        .await
        .is_err()
    {
        tracing::warn!("search catalog refresh failed; starting with an empty catalog");
    }
    let search_deps = functions::search::Deps {
        config: cfg_handle.clone(),
        catalog: search_catalog.clone(),
        sessions: Arc::default(),
        registry_cache: registry_cache.clone(),
        semantic: semantic.clone(),
        judge: functions::search_judge::JudgeSearch::new(iii.clone()),
        registered_workers: Some(registered_cache.clone()),
        iii: Some(iii.clone()),
    };
    functions::search::register(&iii, &search_deps);
    // The same model, as router::embed's local embedder.
    functions::embed_provider::register(&iii, &semantic);
    functions::search::observe_catalog_changes(&iii);
    // The judge's context window (compact options, tournament) follows the
    // hub's default provider: re-read it as soon as the hub's configuration
    // changes instead of when the cache expires.
    let judge = search_deps.judge.clone();
    let follower = iii.clone();
    tokio::spawn(async move {
        match iii_config_client::follow(
            &follower,
            "judge",
            "directory::on-judge-config-change",
            "Internal: re-read the judge's context window when the judge hub's configuration changes.",
        )
        .await
        {
            Ok(mut hub) => {
                while hub.changed().await.is_ok() {
                    judge.forget_window();
                }
            }
            Err(reason) => {
                tracing::debug!(reason, "judge hub configuration not followed; the window cache expires on its own")
            }
        }
    });
    if download_bundle {
        let root = function_search_model_path.clone().expect("checked above");
        let catalog = search_catalog.clone();
        tokio::spawn(async move {
            tracing::info!(
                path = %root.display(),
                "downloading the pinned MiniLM search bundle in the background (first run, \
                 ~180 MB, every file verified by length and SHA-256); BM25 serves meanwhile"
            );
            match functions::search_semantic::download_bundle(&root).await {
                Ok(()) => {
                    tracing::info!(path = %root.display(), "MiniLM search bundle ready");
                    let tools = catalog.read().await;
                    semantic.rebuild(tools.clone());
                }
                Err(error) => tracing::warn!(
                    %error,
                    path = %root.display(),
                    "MiniLM search bundle download failed; directory::search_functions runs \
                     BM25-only until the bundle is provisioned"
                ),
            }
        });
    }
    let hint_observer = iii_directory::hook::HintObserverSlot::default();
    iii_directory::hook::apply(&iii, &hint_observer, cfg_handle.load().inject_hint);

    // Injectable console UI: the directory editor page, the
    // directory::* function-call renderer, and the configuration form.
    iii_directory::ui::register(&iii);

    // Auto-download: observe worker connections + boot reconcile. Wired
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

    // Observe configuration changes so tunable fields hot-reload.
    let state = configuration::SharedState::new(
        cfg_handle.clone(),
        cache_ttl_ms,
        registry_cache,
        registered_cache,
        boot_topology,
        hint_observer,
        search_deps.clone(),
    );
    configuration::observe_config_changes(&iii, state);

    // Filesystem watch: external edits under the directory roots call the
    // matching on-change with { op: "external" }. Failure to start is a
    // degradation (tabs refresh on interaction only), never fatal.
    let cfg_now = cfg_handle.load_full();
    let mut watch_roots = vec![iii_directory::watch::WatchRoot {
        path: cfg_now.resolved_skills_folder(),
        kind: iii_directory::watch::WatchRootKind::Directory,
    }];
    let local_root = cfg_now.local_skills_folder();
    if local_root != watch_roots[0].path {
        watch_roots.push(iii_directory::watch::WatchRoot {
            path: local_root,
            kind: iii_directory::watch::WatchRootKind::Directory,
        });
    }
    // Profile roots: the read-write project root is always watched; the
    // user-global `~/.iii/agents` root only when it already exists — this
    // worker edits profiles there in place but never materializes the
    // directory itself. A directory already watched under another kind is
    // added again under this one: its edits must reach both change functions.
    for (i, profiles_root) in cfg_now.resolved_agents_roots().into_iter().enumerate() {
        if i > 0 && !profiles_root.is_dir() {
            continue;
        }
        watch_roots.push(iii_directory::watch::WatchRoot {
            path: profiles_root,
            kind: iii_directory::watch::WatchRootKind::AgentProfiles,
        });
    }
    // The agent-skills roots (project + user-global) are watched only when
    // they already exist. This worker must never materialize (or write)
    // `.agents/skills` / `~/.agents/skills` — they are owned by external
    // agent tooling.
    for agent_skills_root in cfg_now.resolved_agents_skills_roots() {
        if agent_skills_root.is_dir() {
            watch_roots.push(iii_directory::watch::WatchRoot {
                path: agent_skills_root,
                kind: iii_directory::watch::WatchRootKind::AgentSkills,
            });
        }
    }
    let watch_iii = iii.clone();
    let rt = tokio::runtime::Handle::current();
    let _fs_watch = match iii_directory::watch::spawn_fs_watch(watch_roots, move |kind| {
        let source = match kind {
            iii_directory::fs_source::SourceKind::Skill => sources.skills,
            iii_directory::fs_source::SourceKind::SystemPrompt => sources.system_prompts,
            iii_directory::fs_source::SourceKind::Agent => sources.agents,
        };
        let iii = watch_iii.clone();
        rt.spawn(async move {
            events::emit(&iii, source, serde_json::json!({ "op": "external" })).await;
        });
    }) {
        Ok(h) => {
            tracing::info!("fs watch active on directory roots");
            Some(h)
        }
        Err(e) => {
            tracing::warn!(error = %e, "fs watch disabled");
            None
        }
    };

    tracing::info!("iii-directory ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-directory shutting down");
    iii.shutdown_async().await;
    Ok(())
}

/// The `engine::on-workers-changed` event, delivered as output to
/// `directory::__on_worker_added`. Only `worker_id` is read: the handler
/// looks the worker's name up in `engine::workers::list`. Declared as a struct
/// so the function publishes a typed schema.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct WorkerConnectedEvent {
    #[serde(default)]
    worker_id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct WorkerAddedAck {
    ok: bool,
}

/// The engine's worker connection events.
const WORKERS_ON_CHANGED_FN: &str = "engine::on-workers-changed";

/// Register the internal `directory::__on_worker_added` handler and observe
/// `engine::on-workers-changed` with it for `worker_connected` events.
fn setup_auto_download(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    cache: &Arc<RegisteredWorkersCache>,
    in_flight: &Arc<InFlightGuard>,
) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let cache_inner = cache.clone();
    let in_flight_inner = in_flight.clone();

    // The internal handler that runs when a worker connects.
    iii.register_function(
        "directory::__on_worker_added",
        RegisterFunction::new_async(move |event: WorkerConnectedEvent| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.load_full();
            let cache = cache_inner.clone();
            let in_flight = in_flight_inner.clone();
            async move {
                handle_worker_connected(&iii, &cfg, &cache, &in_flight, &event).await;
                Ok::<_, Error>(WorkerAddedAck { ok: true })
            }
        })
        .description("Internal: auto-download skills when a worker connects.")
        .metadata(serde_json::json!({ "internal": true })),
    );

    // The observer stays `pending` until the engine source is registered,
    // and lives as long as the worker's connection.
    let handle = iii.observe(
        Source::function(WORKERS_ON_CHANGED_FN)
            .filters(json!({ "input.event": { "$eq": "worker_connected" } }))
            .deliver(Deliver::Output),
        "directory::__on_worker_added",
    );
    tracing::info!(
        observer_id = handle.id(),
        "observing worker connections for auto-download"
    );
}

/// The name of the connected worker `worker_id`, from `engine::workers::list`.
async fn connected_worker_name(iii: &IIIClient, worker_id: &str) -> Option<String> {
    let reply = iii
        .call(CallRequest {
            function_id: "engine::workers::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(e) => {
            tracing::warn!(worker_id, error = %e, "engine::workers::list failed; skipping auto-download");
            return None;
        }
    };
    worker_name_in_list(&reply, worker_id)
}

/// The `name` of the `engine::workers::list` row whose `id` is `worker_id`.
fn worker_name_in_list(reply: &Value, worker_id: &str) -> Option<String> {
    reply
        .get("workers")?
        .as_array()?
        .iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(worker_id))?
        .get("name")?
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
}

/// Handle a worker connection. Downloads the worker's skills when they are
/// missing (no local override, no completion marker) and no download for it
/// is already in flight. A worker reconnecting with its skills in place costs
/// one `engine::workers::list` call.
async fn handle_worker_connected(
    iii: &IIIClient,
    cfg: &SkillsConfig,
    cache: &RegisteredWorkersCache,
    in_flight: &Arc<InFlightGuard>,
    event: &WorkerConnectedEvent,
) {
    let Some(worker_id) = event.worker_id.as_deref() else {
        tracing::debug!("worker connection event missing 'worker_id'; skipping");
        return;
    };
    let Some(worker) = connected_worker_name(iii, worker_id).await else {
        tracing::debug!(worker_id, "connected worker has no listed name; skipping");
        return;
    };
    let Some(spec) = reconcile_decision(
        &worker,
        None,
        &cfg.local_skills_folder(),
        &cfg.resolved_skills_folder(),
    ) else {
        tracing::debug!(worker = %worker, "worker skills present or overridden; skipping");
        return;
    };

    // RAII: _claim drops at scope end (including on panic/early-return).
    let Some(_claim) = in_flight.claim(&worker) else {
        tracing::debug!(worker = %worker, "worker download already in-flight; skipping");
        return;
    };

    match download_worker_skills(cfg, &worker, &spec).await {
        Ok(true) => {
            tracing::info!(worker = %worker, "auto-download complete on worker connect");
            cache.invalidate().await;
        }
        Ok(false) => {
            tracing::debug!(worker = %worker, "no skills bundle for worker (404)");
        }
        Err(e) => {
            tracing::warn!(worker = %worker, error = %e, "auto-download failed on worker connect");
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

/// Fetch the compose project's worker (container) names, retrying with
/// backoff while the engine's worker-manager — which registers
/// `compose::status` — is still coming up. On a cold engine start the
/// worker-manager registers late, so `compose::status` reports
/// `function_not_found` for the first few seconds; without a retry the
/// boot reconcile would skip every worker. `compose::status` replaced
/// `worker::list` in iii 0.23 and is registered in this worker's own
/// namespace, so no namespace override.
///
/// Returns the container names on success, or `None` if `compose::status`
/// never became available within the retry budget (~30s). `compose::status`
/// carries no per-worker version, so reconcile pulls the latest skill tag.
async fn fetch_worker_names_with_retry(iii: &IIIClient) -> Option<Vec<String>> {
    const MAX_ATTEMPTS: u32 = 6;
    for attempt in 1..=MAX_ATTEMPTS {
        let result = iii.call(compose_status_request(10_000)).await;

        match result {
            Ok(val) => {
                return match parse_worker_names(&val) {
                    Some(names) => Some(names.into_iter().collect()),
                    None => {
                        tracing::warn!(
                            "boot reconcile: compose::status returned no `containers` array; skipping worker reconcile"
                        );
                        None
                    }
                };
            }
            Err(e) if attempt == MAX_ATTEMPTS => {
                tracing::warn!(
                    attempt,
                    error = %e,
                    "boot reconcile: compose::status unavailable after retries; skipping worker reconcile"
                );
                return None;
            }
            Err(e) => {
                tracing::debug!(
                    attempt,
                    error = %e,
                    "boot reconcile: compose::status not ready (worker-manager still coming up); retrying"
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
        // not a worker, so it never appears in `compose::status`; reconcile
        // it directly (registry pull), independent of — and before — the
        // worker list, so it lands even when `compose::status` isn't ready
        // yet on a cold start.
        if let Some(spec) = reconcile_decision(ENGINE_NAMESPACE, None, &local_root, &global_root) {
            if reconcile_one(&cfg, &in_flight, ENGINE_NAMESPACE, &spec).await {
                reconciled += 1;
            }
        }

        // Retry `compose::status` with backoff: the worker-manager that
        // provides it registers late on a cold engine start. The engine
        // skill was already reconciled above, independent of this call.
        let workers = match fetch_worker_names_with_retry(&iii).await {
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

        for name in &workers {
            // `compose::status` carries no worker version; reconcile the
            // latest skill tag.
            let spec = match reconcile_decision(name, None, &local_root, &global_root) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_name_comes_from_the_listed_row_with_that_id() {
        let reply = json!({ "workers": [
            { "id": "w-1", "name": "kanban" },
            { "id": "w-2", "name": "iii-directory" },
            { "id": "w-3", "name": "  " },
            { "id": "w-4" },
        ] });
        assert_eq!(
            worker_name_in_list(&reply, "w-2").as_deref(),
            Some("iii-directory")
        );
        assert_eq!(worker_name_in_list(&reply, "w-3"), None);
        assert_eq!(worker_name_in_list(&reply, "w-4"), None);
        assert_eq!(worker_name_in_list(&reply, "w-9"), None);
        assert_eq!(worker_name_in_list(&json!({}), "w-1"), None);
    }
}
