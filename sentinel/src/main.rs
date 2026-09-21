//! Boot.
//!
//! Order matters and is asserted by a test: the **interface** — functions,
//! trigger types, the configuration doorbell — registers before the durable
//! dependencies are claimed. Release interface capture boots this binary
//! against an isolated engine that has no `database` and no `queue` and
//! expects a non-empty surface within seconds, so claiming the store first
//! would hang the capture and hide the worker from the registry.
//!
//! The dependencies are then claimed in the background, and ingest stays
//! closed until they answer (`sentinel::status.enabled`). A recovery loop
//! reconciles the trigger bindings every thirty seconds, because a monitor
//! that quietly stopped watching is worse than one that never started.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use sentinel::events::{Emitter, Subscribers};
use sentinel::iii_runtime::harness::{IiiEngineWindow, IiiHarness};
use sentinel::iii_runtime::{Checkouts, IiiDb, IiiRegistry, IiiTelemetry, IngestQueue, Runtime};
use sentinel::ingest::{ring::PhantomRing, Ingest};
use sentinel::investigation::proxies::Proxies;
use sentinel::investigation::Investigations;
use sentinel::registry::Registry;
use sentinel::service::Service;
use sentinel::store::Store;
use sentinel::triggers::Bindings;
use sentinel::{configuration, dependencies, events, functions, manifest, Counters, IngestJob};
use tokio::sync::RwLock;

/// How often the bindings are reconciled and stale work is swept.
const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);
/// How often parked logs are checked for an expired wait.
const JOIN_SWEEP_INTERVAL: Duration = Duration::from_millis(500);
/// Parked logs promoted per sweep.
const JOIN_SWEEP_BATCH: usize = 100;
/// Ingest jobs in flight. Four traces at a time keeps up with a burst
/// without turning it into database contention.
const INGEST_CONCURRENCY: u32 = 4;

#[derive(Debug, Parser)]
#[command(
    name = "sentinel",
    about = "Error monitoring for iii: group failures, freeze the evidence, investigate with the harness"
)]
struct Cli {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())
                .expect("manifest must serialize")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: sentinel::WORKER_NAME.into(),
                os: std::env::consts::OS.into(),
                description: Some(manifest::DESCRIPTION.into()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            // Named explicitly: the SDK would otherwise report the binary's
            // file name as `service.name`, and this worker attributes errors
            // by service name — including its own, which it must recognise to
            // avoid ingesting itself.
            otel: Some(OtelConfig {
                service_name: Some(sentinel::WORKER_NAME.into()),
                service_version: Some(env!("CARGO_PKG_VERSION").into()),
                ..OtelConfig::default()
            }),
            ..InitOptions::default()
        },
    ));

    let (config, config_error) = configuration::register_and_fetch_until_ready(&iii).await;
    let cell: sentinel::ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
    let error_cell: sentinel::ConfigErrorCell = Arc::new(RwLock::new(config_error));
    let counters = Arc::new(Counters::default());
    let ring = Arc::new(PhantomRing::default());
    let runtime = Runtime::new(iii.clone(), ring.clone());

    let database = cell.read().await.database.clone();
    let store = Arc::new(Store::new(IiiDb::new(runtime.clone(), database)));
    let telemetry = Arc::new(IiiTelemetry::new(runtime.clone()));
    let registry = Arc::new(Registry::new(
        IiiRegistry::new(runtime.clone()),
        Duration::from_millis(cell.read().await.workers_ttl_ms),
    ));
    let queue = Arc::new(IngestQueue::new(runtime.clone(), INGEST_CONCURRENCY));
    let subscribers = Subscribers::default();
    let emitter = Emitter::new(iii.clone(), subscribers.clone());
    let checkouts = Arc::new(Checkouts::new(cell.clone()));
    let investigations = Arc::new(Investigations::new(
        store.clone(),
        Arc::new(IiiHarness::new(runtime.clone())),
        checkouts.clone(),
        cell.clone(),
    ));
    let proxies = Arc::new(Proxies::new(
        Arc::new(IiiEngineWindow::new(runtime.clone())),
        cell.clone(),
    ));

    let deps = Arc::new(functions::Deps {
        config: cell.clone(),
        config_error: error_cell.clone(),
        counters: counters.clone(),
        store: store.clone(),
        service: Arc::new(Service::new(store.clone(), telemetry.clone())),
        ingest: Arc::new(Ingest::new(
            store.clone(),
            telemetry.clone(),
            registry.clone(),
            checkouts,
            counters.clone(),
            ring.clone(),
        )),
        investigations: investigations.clone(),
        proxies,
        queue: queue.clone(),
        emitter,
    });

    events::register_trigger_types(&iii, &subscribers);
    functions::register_all(&iii, &deps);

    let bindings = Bindings::default();
    match configuration::bind_reload(&iii, cell.clone(), error_cell.clone()) {
        // One extra pass closes the boot gap: an update that landed between
        // the fetch above and this binding fired into nothing.
        Ok(reload) => reload.run().await,
        Err(error) => tracing::warn!(
            %error,
            "configuration updates are not bound; settings changes need a restart until it recovers"
        ),
    }

    let readiness = tokio::spawn(claim_dependencies(
        iii.clone(),
        store.clone(),
        queue.clone(),
        counters.clone(),
        cell.clone(),
        bindings.clone(),
        deps.clone(),
    ));
    let sweeper = tokio::spawn(sweep_parked_logs(
        store.clone(),
        queue.clone(),
        counters.clone(),
    ));

    tracing::info!(
        config_id = configuration::config_id(),
        "sentinel ready; ingest opens when the store and queue answer"
    );
    tokio::signal::ctrl_c().await?;
    readiness.abort();
    sweeper.abort();
    iii.shutdown_async().await;
    Ok(())
}

/// Claim the durable dependencies, then keep the bindings alive and the
/// investigations honest.
#[allow(clippy::too_many_arguments)]
async fn claim_dependencies(
    iii: Arc<IIIClient>,
    store: Arc<Store<IiiDb>>,
    queue: Arc<IngestQueue>,
    counters: Arc<Counters>,
    config: sentinel::ConfigCell,
    bindings: Bindings,
    deps: Arc<functions::Deps<IiiRegistry>>,
) {
    dependencies::claim(store, queue, counters).await;

    let mut interval = tokio::time::interval(RECOVERY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let snapshot = config.read().await.clone();
        bindings.reconcile(&iii, &snapshot).await;
        // A doorbell that never rang — the harness restarted, the trigger was
        // not bound yet — would otherwise leave a first pass running forever.
        let events = deps.investigations.reconcile().await;
        functions::broadcast(&deps, events).await;
        let (trace, log) = bindings.bound();
        if snapshot.enabled
            && ((snapshot.sources.trace.enabled && !trace)
                || (snapshot.sources.log.enabled && !log))
        {
            tracing::warn!(
                trace_bound = trace,
                log_bound = log,
                "a source is enabled but not bound; retrying on the next pass"
            );
        }
        interval.tick().await;
    }
}

/// Promote the logs whose wait for a span has run out.
async fn sweep_parked_logs(
    store: Arc<Store<IiiDb>>,
    queue: Arc<IngestQueue>,
    counters: Arc<Counters>,
) {
    let mut interval = tokio::time::interval(JOIN_SWEEP_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if !counters.is_ready() {
            continue;
        }
        let due = match store
            .due_pending_logs(sentinel::ids::now_ms(), JOIN_SWEEP_BATCH)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "could not read the parked logs");
                continue;
            }
        };
        for pending in due {
            let job = IngestJob::Promote {
                trace_id: pending.trace_id.clone().unwrap_or_else(|| "-".into()),
                occurrence_id: pending.id.clone(),
            };
            if let Err(error) = queue.enqueue(&job).await {
                tracing::warn!(%error, "could not queue a parked log for promotion");
            } else {
                counters.add_queued(1);
            }
        }
    }
}
