//! `memory` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file is only a SEED for the
//!      first registration; the authoritative config lives in the
//!      `configuration` worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative
//!      value. `configuration` is a required boot dependency.
//!   4. Open the store. An unwritable data_dir is BOOT-FATAL: a memory
//!      worker that silently runs in RAM and prints "persisted" is the
//!      worst failure mode a memory product can have.
//!   5. Register the two change functions (`memory::on-item-changed`,
//!      `memory::on-bank-changed`), then the `memory::*` functions and the
//!      internal hook handlers.
//!   6. Observe the harness seams (`harness::hook::on-pre-generate`
//!      fail-OPEN — memory must never block a turn — and
//!      `harness::on-turn-completed` for extraction), `session::on-deleted`
//!      and the extraction queue's `queue::on-message`, the last under a
//!      stable id ([`hooks::extraction_observer_id`]) so a restarted worker
//!      consumes the jobs its previous process left unacked. An observer
//!      stays `pending` until its function registers, so in a deployment
//!      without the harness the worker still boots and serves its RPCs.
//!   7. Observe `configuration::on-updated` LAST.
//!   8. Sleep on Ctrl+C, then `shutdown_async` cleanly. There is no
//!      shutdown flush: every write was already fsynced at commit time.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::protocol::observe_codes;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, ObserverHandle, ObserverOptions,
    RegisterFunction, Source,
};
use serde_json::json;
use tokio::sync::RwLock;

use memory::configuration::{self, ConfigCell, StoreCell};
use memory::deps::Deps;
use memory::events::{self, Emitter};
use memory::functions::rest;
use memory::store::Store;
use memory::{config, functions, hooks, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "memory",
    about = "Durable cross-session agent memory: named banks, always-injected rules, auto-extracted memories, hybrid recall."
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on the
    /// first registration. The AUTHORITATIVE config is always fetched from
    /// the `configuration` worker afterward; this file only seeds it.
    #[arg(long)]
    config: Option<String>,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: hooks::WORKER_NAME.to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// The function `http::on-request` routes REST calls through.
const HTTP_ON_REQUEST_FN: &str = "http::on-request";

/// First wait before observing a seam again after `OBSERVER_ID_CONFLICT`;
/// it doubles up to [`REOBSERVE_MAX_SECS`].
const REOBSERVE_FIRST_SECS: u64 = 1;
const REOBSERVE_MAX_SECS: u64 = 30;

/// The functions this worker observes, each with its target and observer
/// options. Every observer stays `pending` until its function registers and
/// its host accepts it, so the harness, session-manager and queue can boot
/// in any order. The extraction consumer observes under `extraction_id`
/// ([`hooks::extraction_observer_id`]).
fn seam_observers(extraction_id: String) -> Vec<(Source, &'static str, ObserverOptions)> {
    vec![
        (
            Source::function("harness::hook::on-pre-generate")
                .args(json!({ "priority": 100, "timeout_ms": 3_000, "on_error": "fail_open" })),
            hooks::PRE_GENERATE_FN,
            ObserverOptions::default(),
        ),
        (
            Source::function("harness::on-turn-completed").deliver(Deliver::Output),
            hooks::TURN_COMPLETED_FN,
            ObserverOptions::default(),
        ),
        (
            Source::function("session::on-deleted").deliver(Deliver::Output),
            hooks::SESSION_DELETED_FN,
            ObserverOptions::default(),
        ),
        (
            Source::function("queue::on-message").args(json!({
                "topic": hooks::EXTRACTION_QUEUE,
                "max_retries": 3,
                "backoff_ms": 2_000,
            })),
            hooks::EXTRACT_JOB_FN,
            ObserverOptions::default().id(extraction_id),
        ),
    ]
}

/// Observes `source` now, in the worker's observer order, and keeps the
/// observer from a task of its own ([`keep_observed`]).
fn observe_seam(
    iii: &Arc<IIIClient>,
    source: Source,
    target: &'static str,
    options: ObserverOptions,
) {
    let observer = observe_logged(iii, &source, target, &options);
    tokio::spawn(keep_observed(
        iii.clone(),
        observer,
        source,
        target,
        options,
    ));
}

fn observe_logged(
    iii: &IIIClient,
    source: &Source,
    target: &'static str,
    options: &ObserverOptions,
) -> ObserverHandle {
    let observer = iii.observe_with(source.clone(), target, options.clone());
    tracing::info!(
        function_id = source.function_id(),
        target,
        observer_id = observer.id(),
        "observing"
    );
    observer
}

/// Keeps `observer`. It stays `pending` until its function registers; a
/// refusal is logged. `OBSERVER_ID_CONFLICT` (the extraction consumer's
/// stable id still held: by a connection of this worker's previous process
/// the engine has not seen close yet, or by another worker until the
/// engine's grace period, `observers.observer_grace_ms`, ends) disposes the
/// refused observer and observes again after [`REOBSERVE_FIRST_SECS`], then
/// twice as long each time up to [`REOBSERVE_MAX_SECS`].
async fn keep_observed(
    iii: Arc<IIIClient>,
    mut observer: ObserverHandle,
    source: Source,
    target: &'static str,
    options: ObserverOptions,
) {
    let mut retry = std::time::Duration::from_secs(REOBSERVE_FIRST_SECS);
    loop {
        let Err(error) = observer.ready().await else {
            tracing::debug!(
                function_id = source.function_id(),
                observer_id = observer.id(),
                "observer active"
            );
            return;
        };
        let code = observer.reason().map(|reason| reason.code);
        let held = code.as_deref() == Some(observe_codes::OBSERVER_ID_CONFLICT);
        tracing::warn!(
            function_id = source.function_id(),
            target,
            observer_id = observer.id(),
            error = %error,
            retry_in_secs = held.then_some(retry.as_secs()),
            "observer refused"
        );
        if !held {
            return;
        }
        // The SDK keeps a refused observer and replays it until disposed.
        let _ = observer.dispose().await;
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(std::time::Duration::from_secs(REOBSERVE_MAX_SECS));
        observer = observe_logged(&iii, &source, target, &options);
    }
}

/// The REST route of a public function: `POST /memory/...`, the function id
/// with `::` as `/`. Its target is `memory::rest`.
fn rest_route(function_id: &str) -> Source {
    Source::function(HTTP_ON_REQUEST_FN).args(json!({
        "method": "POST",
        "path": rest::route_path(function_id),
    }))
}

/// Ensure the extraction queue exists. A pending observer covers the queue
/// consumer, but `queue::define` is an RPC the queue worker must be up to
/// serve — enqueue rejects an undefined queue — so poll until the
/// (idempotent) definition lands.
fn ensure_extraction_queue(iii: Arc<IIIClient>) {
    tokio::spawn(async move {
        loop {
            let defined = iii
                .call(CallRequest {
                    function_id: "queue::define".into(),
                    payload: json!({ "queue": hooks::EXTRACTION_QUEUE, "config": {} }),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await
                .is_ok();
            if defined {
                tracing::info!("memory extraction queue defined");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
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

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative value comes from
    // the configuration worker). The seed IS env-expanded (`${VAR}`).
    let seed = match cli.config.as_deref() {
        Some(path) => match config::WorkerConfig::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on the stored configuration entry"
                );
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering memory configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading memory configuration")?;

    // Boot-fatal on an unwritable data_dir: never silently run in RAM.
    let store = Store::open(cfg.resolved_data_dir())
        .map_err(|e| anyhow::anyhow!("opening the memory store: {e}"))?;
    let store_cell: StoreCell = Arc::new(RwLock::new(Arc::new(store)));

    // The change functions first: every write path calls them.
    events::register_all(&iii);
    let emitter = Arc::new(Emitter::new(iii.clone()));

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        store: store_cell.clone(),
        config: cell.clone(),
        emitter,
    });

    functions::register_all(&iii, &deps);
    rest::register(&iii, &deps);
    register_hook_functions(&iii, &deps);

    // Injectable console UI — after the memory::* functions so the console can
    // attribute the assets. `iii` is already Arc-wrapped (the console-ui crate
    // clones the client into its hot-reload watcher task).
    memory::ui::register(&iii);

    // Observe the harness/session/queue seams (injection fail-OPEN — a
    // memory failure must never block a turn — plus turn-completed
    // extraction, session GC, and the durable extraction consumer). If an
    // owning sibling boots after this worker, its observer stays pending and
    // becomes active once the sibling registers the function. The observers
    // live as long as the worker's connection. The extraction consumer's
    // stable id keeps its unacked jobs across a restart.
    let extraction_id = hooks::extraction_observer_id(
        iii.namespace().as_deref(),
        &hooks::worker_name(),
        hooks::EXTRACTION_QUEUE,
    );
    for (source, target, options) in seam_observers(extraction_id) {
        observe_seam(&iii, source, target, options);
    }
    ensure_extraction_queue(iii.clone());

    // Catch-up vector backfill for memories saved while embeddings were
    // unavailable (delayed so the router and providers finish booting).
    memory::embed_client::backfill_all(deps.clone());

    // Reuse the http worker: every public function is also a REST route
    // (`POST /memory/...`) served by `memory::rest`, which takes the
    // HttpRequest and answers { status_code, body }. Without the http worker
    // the routes stay pending and the bus surface still serves everything.
    for spec in functions::catalog() {
        let observer = iii.observe(rest_route(spec.function_id), rest::REST_ID);
        tracing::debug!(
            function_id = spec.function_id,
            observer_id = observer.id(),
            "REST route observed"
        );
    }

    // LAST: observe configuration changes so the handler closes over the
    // fully-built cells.
    configuration::observe_config_changes(&iii, cell, store_cell);

    tracing::info!(
        "memory ready: 17 memory::* functions + 2 change functions + REST via http::on-request"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("memory shutting down (all writes already fsynced)");
    iii.shutdown_async().await;
    Ok(())
}

/// The two internal harness-facing handlers. Registered like every other
/// function so hook invocations show up in traces, but marked internal.
fn register_hook_functions(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let d = deps.clone();
    iii.register_function(
        hooks::PRE_GENERATE_FN,
        RegisterFunction::new_async(move |input: hooks::PreGenerateInput| {
            let d = d.clone();
            async move { hooks::pre_generate(&d, input).await }
        })
        .description(hooks::PRE_GENERATE_DESC)
        // Deliberately NOT trace_hidden: seeing memory feed each turn (which
        // bank, which memories) in the trace timeline is a product requirement,
        // not plumbing noise.
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |input: hooks::TurnCompletedInput| {
            let d = d.clone();
            async move { hooks::turn_completed(&d, input).await }
        })
        .description(hooks::TURN_COMPLETED_DESC)
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::EXTRACT_JOB_FN,
        RegisterFunction::new_async(move |input: hooks::ExtractJobInput| {
            let d = d.clone();
            async move { hooks::extract_job(&d, input).await }
        })
        .description(hooks::EXTRACT_JOB_DESC)
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::SESSION_DELETED_FN,
        RegisterFunction::new_async(move |input: hooks::SessionDeletedInput| {
            let d = d.clone();
            async move { hooks::session_deleted(&d, input).await }
        })
        .description(hooks::SESSION_DELETED_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_routes_take_the_leading_slash() {
        assert_eq!(
            rest_route("memory::save"),
            Source::function("http::on-request")
                .args(json!({ "method": "POST", "path": "/memory/save" }))
        );
        assert_eq!(
            rest_route("memory::bank::create"),
            Source::function("http::on-request")
                .args(json!({ "method": "POST", "path": "/memory/bank/create" }))
        );
    }

    #[test]
    fn seams_observe_the_harness_session_and_queue_functions() {
        let observed: Vec<(String, &str)> = seam_observers("fixed".into())
            .into_iter()
            .map(|(source, target, _)| (source.function_id().to_string(), target))
            .collect();
        assert_eq!(
            observed,
            vec![
                (
                    "harness::hook::on-pre-generate".to_string(),
                    hooks::PRE_GENERATE_FN
                ),
                (
                    "harness::on-turn-completed".to_string(),
                    hooks::TURN_COMPLETED_FN
                ),
                ("session::on-deleted".to_string(), hooks::SESSION_DELETED_FN),
                ("queue::on-message".to_string(), hooks::EXTRACT_JOB_FN),
            ]
        );
    }

    #[test]
    fn each_seam_target_describes_the_function_it_observes() {
        let description = |target: &str| match target {
            hooks::PRE_GENERATE_FN => hooks::PRE_GENERATE_DESC,
            hooks::TURN_COMPLETED_FN => hooks::TURN_COMPLETED_DESC,
            hooks::SESSION_DELETED_FN => hooks::SESSION_DELETED_DESC,
            hooks::EXTRACT_JOB_FN => hooks::EXTRACT_JOB_DESC,
            other => panic!("no description for {other}"),
        };
        for (source, target, _) in seam_observers("fixed".into()) {
            let text = description(target);
            assert!(
                text.contains(&format!("target of the {} observer", source.function_id())),
                "{target}: {text}"
            );
            for removed in [
                "harness::hook::pre-generate",
                "harness::turn-completed",
                "session::deleted",
                "durable:subscriber",
            ] {
                assert!(!text.contains(removed), "{target} names {removed}: {text}");
            }
        }
    }

    #[test]
    fn the_extraction_consumer_names_its_topic_under_the_stable_id() {
        let mut seams = seam_observers("fixed".into());
        let (queue, _, options) = seams.pop().unwrap();
        assert_eq!(
            queue,
            Source::function("queue::on-message").args(json!({
                "topic": hooks::EXTRACTION_QUEUE,
                "max_retries": 3,
                "backoff_ms": 2_000,
            }))
        );
        assert_eq!(options, ObserverOptions::default().id("fixed"));
        // The other seams carry no backlog: each process observes them
        // under ids of its own.
        assert!(seams
            .iter()
            .all(|(_, _, options)| *options == ObserverOptions::default()));
    }
}
