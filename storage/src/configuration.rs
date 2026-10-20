//! Integration with the `configuration` worker — registration, authoritative
//! reads, and live application of every storage setting.
//!
//! Buckets are built one by one: a bucket whose backend cannot be built (an
//! unresolvable `secret://` reference, a bad field, rejected credentials) is
//! listed with its error and answers `BUCKET_UNAVAILABLE`, while every other
//! bucket keeps working. A reload rebuilds only the buckets whose entry or
//! resolved secrets changed (cloud clients and their connection pools are
//! kept), and keeps the notification pollers whose sources did not change.
//! The native-local listener stays transactional: a `providers.local` that
//! cannot be applied (a port already taken) rejects the whole update and the
//! running generation stays.

use crate::backend::factory::{self, LocalBackendCtx, Purpose};
use crate::backend::local::LocalRuntime;
use crate::backend::Backend;
use crate::config::{BucketConfig, WorkerConfig};
use crate::handlers::{AppState, Catalog};
use crate::material::{self, BucketError, Material, Stage};
use crate::secrets::Refetch;
use crate::triggers::dispatcher::EngineDispatcher;
use crate::triggers::handler::WiredBuckets;
use crate::triggers::pollers::{cf_queue, pubsub, sqs};
use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub const DEFAULT_CONFIG_ID: &str = "storage";

/// The configuration entry this worker owns.
///
/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name. A worker
/// that hardcodes its id turns that id into a global scarce name: two instances
/// share one entry and take turns overwriting it, and each write wakes both.
/// Being told which entry is its own is what lets them differ.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}
const CONFIG_FN_ID: &str = "storage::on-config-change";
const SECRET_FN_ID: &str = "storage::on-secret-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// How often the maintenance tick retries failed buckets and re-reads
/// secret values past their TTL.
pub const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(15);

/// Where the storage configuration value lives: the configuration worker in
/// the worker, an in-memory value in tests.
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// The stored value: `raw` as written (`${VAR}` templates and
    /// references unexpanded), else env-expanded. `Null` when nothing is
    /// stored.
    async fn get(&self, raw: bool) -> Result<Value, String>;
    /// Replace the whole stored value (validated against the registered
    /// schema, then announced to every consumer).
    async fn set(&self, value: Value) -> Result<(), String>;
}

/// [`ConfigStore`] over the configuration worker.
pub struct EngineStore {
    iii: IIIClient,
}

impl EngineStore {
    pub fn new(iii: IIIClient) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl ConfigStore for EngineStore {
    async fn get(&self, raw: bool) -> Result<Value, String> {
        let mut payload = json!({ "id": config_id() });
        if raw {
            payload["raw"] = json!(true);
        }
        match trigger_configuration_with_retry(&self.iii, "configuration::get", payload).await {
            Ok(resp) => Ok(resp.get("value").cloned().unwrap_or(Value::Null)),
            Err(error) if is_not_found(&error) => Ok(Value::Null),
            Err(error) => Err(error),
        }
    }

    async fn set(&self, value: Value) -> Result<(), String> {
        // One attempt: a rejected value (schema) must surface at once.
        self.iii
            .trigger(
                TriggerRequest {
                    function_id: "configuration::set".to_string(),
                    payload: json!({ "id": config_id(), "value": value }),
                    action: None,
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The live resources that configuration can replace: native-local service and
/// listener, per-bucket backends, cloud notification pollers, and the trigger
/// registration wiring set. `apply_lock` serializes the complete fetch →
/// prepare → publish flow so rapid edits cannot land out of order; `saves`
/// serializes the bucket editor's read-modify-writes.
pub struct StorageRuntime {
    local: Arc<LocalRuntime>,
    dispatcher: Arc<EngineDispatcher>,
    wired_buckets: WiredBuckets,
    /// Running notification pollers by source key (see [`PollerSpec`]).
    pollers: Mutex<HashMap<String, JoinHandle<()>>>,
    /// What each live cloud backend was built from ([`material::fingerprint`]).
    fingerprints: Mutex<HashMap<String, u64>>,
    apply_lock: Mutex<()>,
    saves: Mutex<()>,
    store: Option<Arc<dyn ConfigStore>>,
}

impl StorageRuntime {
    pub fn new(
        dispatcher: Arc<EngineDispatcher>,
        state: &AppState,
        wired_buckets: WiredBuckets,
    ) -> Arc<Self> {
        Self::with_store(dispatcher, state, wired_buckets, None)
    }

    pub fn with_store(
        dispatcher: Arc<EngineDispatcher>,
        state: &AppState,
        wired_buckets: WiredBuckets,
        store: Option<Arc<dyn ConfigStore>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            local: Arc::new(LocalRuntime::new(
                dispatcher.clone(),
                state.reconfigure_gate.clone(),
            )),
            dispatcher,
            wired_buckets,
            pollers: Mutex::new(HashMap::new()),
            fingerprints: Mutex::new(HashMap::new()),
            apply_lock: Mutex::new(()),
            saves: Mutex::new(()),
            store,
        })
    }

    /// The configuration store, when the runtime has one (not in tests that
    /// only apply snapshots).
    pub fn store(&self) -> Option<&Arc<dyn ConfigStore>> {
        self.store.as_ref()
    }

    /// Held by `saveBucket` / `deleteBucket` across their read-modify-write.
    pub fn saves(&self) -> &Mutex<()> {
        &self.saves
    }

    /// Apply a snapshot (used at boot and by unit tests). Every fallible
    /// shared prerequisite (the local listener) is resolved before the short
    /// publish phase; a bucket that fails is reported, not fatal.
    pub async fn apply_config(&self, state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
        let _apply = self.apply_lock.lock().await;
        self.prepare_and_publish(state, cfg).await
    }

    /// Re-fetch and apply the authoritative stored value while holding the
    /// serialization lock end-to-end. The trigger payload is intentionally not
    /// trusted because the function is discoverable on the bus.
    pub async fn reload(&self, state: &AppState) -> Result<(), String> {
        let Some(store) = self.store.clone() else {
            return Err("no configuration store".into());
        };
        let _apply = self.apply_lock.lock().await;
        let value = store.get(false).await?;
        let cfg = if value.is_null() {
            WorkerConfig::default()
        } else {
            WorkerConfig::from_json_isolating(&value)?
        };
        self.prepare_and_publish(state, cfg).await
    }

    /// Apply the current snapshot again (a secret changed, a failed bucket
    /// is retried). The snapshot is read under the lock, so a newer reload
    /// is never overwritten with an older value.
    pub async fn reapply(&self, state: &AppState) -> Result<(), String> {
        let _apply = self.apply_lock.lock().await;
        let cfg = state.catalog.read().await.config.clone();
        self.prepare_and_publish(state, cfg).await
    }

    async fn prepare_and_publish(&self, state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
        let references: BTreeSet<String> = cfg
            .buckets
            .values()
            .flat_map(material::references)
            .collect();
        state.secrets.ensure(references.iter().cloned()).await;
        state.secrets.retain(&references);

        let needs_local = cfg
            .buckets
            .values()
            .any(|bucket| matches!(bucket, BucketConfig::Local(_)));
        let local_update = self
            .local
            .prepare_update(cfg.providers.local.as_ref(), needs_local)
            .await
            .map_err(|error| format!("preparing native local storage: {error}"))?;

        let current = state.backends_snapshot().await;
        let fingerprints = self.fingerprints.lock().await.clone();
        let built =
            build_buckets(state, &cfg, local_update.context(), &current, &fingerprints).await;

        let previous = state.catalog.read().await.clone();
        let mut new_backends = HashMap::new();
        let mut new_fingerprints = HashMap::new();
        let mut materials = HashMap::new();
        let mut errors = HashMap::new();
        for (name, invalid) in &cfg.invalid {
            errors.insert(
                name.clone(),
                BucketError::new(Stage::Config, invalid.message.clone()),
            );
        }
        for (name, result) in built {
            match result {
                Ok(bucket) => {
                    if !matches!(cfg.buckets.get(&name), Some(BucketConfig::Local(_))) {
                        new_fingerprints.insert(name.clone(), bucket.fingerprint);
                    }
                    new_backends.insert(name.clone(), bucket.backend);
                    materials.insert(name, bucket.material);
                }
                Err(error) => {
                    errors.insert(name, error);
                }
            }
        }
        for (name, error) in &errors {
            // Only a new or changed failure is worth a log line: the
            // maintenance tick retries failed buckets.
            if previous.errors.get(name) != Some(error) {
                tracing::warn!(bucket = %name, stage = error.stage.as_str(), error = %error.message, "bucket unavailable");
            }
        }

        // Notification pollers: keep the running ones whose source is
        // unchanged, prepare the new ones; a source that cannot be prepared
        // leaves its buckets working without object triggers.
        let specs = poller_specs(&cfg, &materials);
        let running: HashSet<String> = self
            .pollers
            .lock()
            .await
            .iter()
            .filter(|(_, handle)| !handle.is_finished())
            .map(|(key, _)| key.clone())
            .collect();
        let mut prepared = Vec::new();
        let mut live_sources = HashSet::new();
        let mut notification_errors = HashMap::new();
        for spec in specs {
            if running.contains(&spec.key) {
                live_sources.insert(spec.key.clone());
                continue;
            }
            match spec.source.prepare().await {
                Ok(poller) => {
                    live_sources.insert(spec.key.clone());
                    prepared.push((spec.key, poller));
                }
                Err(error) => {
                    for bucket in spec.buckets {
                        if previous.notification_errors.get(&bucket) != Some(&error) {
                            tracing::warn!(bucket = %bucket, error = %error, "notifications source unavailable; object triggers will not fire");
                        }
                        notification_errors.insert(bucket, error.clone());
                    }
                }
            }
        }
        let new_wired_buckets = wired_buckets(&cfg, &new_backends, &notification_errors);

        // Requests and signed HTTP handlers take the read side only while
        // selecting an Arc-backed generation. Holding the write side here makes
        // the multi-resource publication atomic from their perspective.
        let _publish = state.reconfigure_gate.write().await;
        self.local.commit(local_update).await;
        *state.backends.write().await = new_backends;
        *self.wired_buckets.write().await = new_wired_buckets;
        {
            let mut pollers = self.pollers.lock().await;
            pollers.retain(|key, handle| {
                let keep = live_sources.contains(key) && !handle.is_finished();
                if !keep {
                    handle.abort();
                }
                keep
            });
            for (key, poller) in prepared {
                pollers.insert(key, poller.spawn(self.dispatcher.clone()));
            }
        }
        *self.fingerprints.lock().await = new_fingerprints;
        let changed = previous.errors != errors
            || previous.notification_errors != notification_errors
            || bucket_names(&previous.config) != bucket_names(&cfg);
        let bucket_count = cfg.buckets.len() + cfg.invalid.len();
        let unavailable = errors.len();
        *state.catalog.write().await = Catalog {
            config: cfg,
            errors,
            notification_errors,
        };
        drop(_publish);

        if let Some(addr) = self.local.current_addr().await {
            tracing::debug!(address = %addr, "native local storage HTTP server ready");
        }
        let notification_sources = self.pollers.lock().await.len();
        if changed {
            tracing::info!(
                buckets = bucket_count,
                unavailable,
                notification_sources,
                "storage configuration applied live"
            );
        } else {
            tracing::debug!(
                buckets = bucket_count,
                unavailable,
                notification_sources,
                "storage configuration re-applied"
            );
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        let _apply = self.apply_lock.lock().await;
        for (_, handle) in self.pollers.lock().await.drain() {
            handle.abort();
        }
        self.local.shutdown().await;
    }

    /// A `secrets::changed` delivery for `reference` (canonical). Whatever
    /// uses it, its cached value is dropped; when a configured bucket uses
    /// it, it is re-read now and the buckets whose resolved values changed
    /// are rebuilt (deleted → those buckets report the error; re-created →
    /// they come back).
    pub async fn on_secret_changed(&self, state: &AppState, reference: &str) {
        let referenced = state
            .catalog
            .read()
            .await
            .config
            .buckets
            .values()
            .any(|bucket| material::references(bucket).iter().any(|r| r == reference));
        if !referenced {
            state.secrets.forget(reference);
            return;
        }
        state
            .secrets
            .refresh([reference.to_string()], Refetch::Invalidate)
            .await;
        if let Err(error) = self.reapply(state).await {
            tracing::warn!(error = %error, "re-applying after a secret change failed");
        }
    }

    /// Retry failed buckets (a secrets worker that came up late, a provider
    /// back online) and re-read secrets past their TTL; a value that changed
    /// rebuilds only its buckets.
    pub fn spawn_maintenance(self: &Arc<Self>, state: AppState) -> JoinHandle<()> {
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(MAINTENANCE_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                let retry = state
                    .catalog
                    .read()
                    .await
                    .errors
                    .values()
                    .any(|error| error.stage != Stage::Config);
                let stale = state.secrets.stale_keys();
                let rotated = !stale.is_empty()
                    && !state
                        .secrets
                        .refresh(stale, Refetch::KeepLastGood)
                        .await
                        .is_empty();
                if retry || rotated {
                    if let Err(error) = runtime.reapply(&state).await {
                        tracing::warn!(error = %error, "maintenance re-apply failed");
                    }
                }
            }
        })
    }
}

fn bucket_names(cfg: &WorkerConfig) -> BTreeSet<&String> {
    cfg.buckets.keys().chain(cfg.invalid.keys()).collect()
}

struct BuiltBucket {
    backend: Arc<dyn Backend>,
    fingerprint: u64,
    material: Material,
}

/// Build every bucket concurrently (a slow provider costs one wait, not
/// one per bucket). A cloud bucket whose fingerprint is unchanged keeps its
/// running backend; local buckets always bind to the new local generation.
async fn build_buckets(
    state: &AppState,
    cfg: &WorkerConfig,
    local_ctx: Option<&LocalBackendCtx>,
    current: &HashMap<String, Arc<dyn Backend>>,
    fingerprints: &HashMap<String, u64>,
) -> Vec<(String, Result<BuiltBucket, BucketError>)> {
    let builds = cfg.buckets.iter().map(|(name, bucket)| async move {
        let result = async {
            let material = Material::resolve(bucket, |key| state.secrets.lookup(key))?;
            let fingerprint = material::fingerprint(name, bucket, &material);
            if !matches!(bucket, BucketConfig::Local(_)) {
                if let (Some(backend), Some(previous)) = (current.get(name), fingerprints.get(name))
                {
                    if *previous == fingerprint {
                        return Ok(BuiltBucket {
                            backend: backend.clone(),
                            fingerprint,
                            material,
                        });
                    }
                }
            }
            let backend = factory::build(
                name,
                bucket,
                &cfg.providers,
                local_ctx,
                &material,
                Purpose::Live,
            )
            .await
            .map_err(|error| BucketError::from_build(error, &material))?;
            tracing::info!(bucket = %name, provider = backend.provider(), "backend ready");
            Ok(BuiltBucket {
                backend,
                fingerprint,
                material,
            })
        }
        .await;
        (name.clone(), result)
    });
    futures_util::future::join_all(builds).await
}

/// Buckets whose object triggers can fire: a live backend, and a running
/// notifications source (local buckets dispatch in-process).
fn wired_buckets(
    cfg: &WorkerConfig,
    backends: &HashMap<String, Arc<dyn Backend>>,
    notification_errors: &HashMap<String, String>,
) -> HashSet<String> {
    cfg.buckets
        .iter()
        .filter(|(name, _)| backends.contains_key(*name))
        .filter(|(name, _)| !notification_errors.contains_key(*name))
        .filter_map(|(name, bucket)| {
            let wired = match bucket {
                BucketConfig::S3(value) => value.notifications.is_some(),
                BucketConfig::Gcs(value) => value.notifications.is_some(),
                BucketConfig::R2(value) => value.notifications.is_some(),
                BucketConfig::Local(_) => true,
            };
            wired.then(|| name.clone())
        })
        .collect()
}

enum PreparedPoller {
    Sqs(sqs::SqsPoller),
    Pubsub(pubsub::PubsubPoller, gcloud_pubsub::client::Client),
    CfQueue(cf_queue::CfQueuePoller),
}

impl PreparedPoller {
    fn spawn(self, dispatcher: Arc<EngineDispatcher>) -> JoinHandle<()> {
        match self {
            Self::Sqs(poller) => tokio::spawn(sqs::run_loop(poller, dispatcher)),
            Self::Pubsub(poller, client) => {
                tokio::spawn(pubsub::run_loop(poller, client, dispatcher))
            }
            Self::CfQueue(poller) => tokio::spawn(cf_queue::run_loop(poller, dispatcher)),
        }
    }
}

/// One notifications source and the buckets it serves.
struct PollerSpec {
    /// Identifies the source and everything it was prepared from (the
    /// Cloudflare token only as a hash): an unchanged key keeps the running
    /// poller.
    key: String,
    buckets: Vec<String>,
    source: PollerSource,
}

enum PollerSource {
    Sqs {
        queue_url: String,
        region: String,
        reverse: BTreeMap<String, String>,
    },
    Pubsub {
        subscription: String,
        reverse: BTreeMap<String, String>,
    },
    CfQueue {
        account_id: String,
        queue_id: String,
        api_token: String,
        reverse: BTreeMap<String, String>,
    },
}

impl PollerSource {
    async fn prepare(self) -> Result<PreparedPoller, String> {
        match self {
            Self::Sqs {
                queue_url,
                region,
                reverse,
            } => {
                let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(region))
                    .load()
                    .await;
                Ok(PreparedPoller::Sqs(sqs::SqsPoller::new(
                    aws_sdk_sqs::Client::new(&sdk_config),
                    queue_url,
                    Arc::new(reverse.into_iter().collect()),
                )))
            }
            Self::Pubsub {
                subscription,
                reverse,
            } => {
                let client_config = gcloud_pubsub::client::ClientConfig::default()
                    .with_auth()
                    .await
                    .map_err(|error| format!("Pub/Sub auth for `{subscription}`: {error}"))?;
                let client = gcloud_pubsub::client::Client::new(client_config)
                    .await
                    .map_err(|error| format!("Pub/Sub client for `{subscription}`: {error}"))?;
                Ok(PreparedPoller::Pubsub(
                    pubsub::PubsubPoller::new(
                        subscription,
                        Arc::new(reverse.into_iter().collect()),
                    ),
                    client,
                ))
            }
            Self::CfQueue {
                account_id,
                queue_id,
                api_token,
                reverse,
            } => {
                cf_queue::probe_auth(&account_id, &queue_id, &api_token)
                    .await
                    .map_err(|error| {
                        let message = error.to_wire_string().replace(&api_token, "***");
                        format!("Cloudflare queue auth for `{queue_id}`: {message}")
                    })?;
                Ok(PreparedPoller::CfQueue(cf_queue::CfQueuePoller::new(
                    account_id,
                    queue_id,
                    api_token,
                    Arc::new(reverse.into_iter().collect()),
                )))
            }
        }
    }
}

/// The notifications sources of the buckets that built, grouped by source.
fn poller_specs(cfg: &WorkerConfig, materials: &HashMap<String, Material>) -> Vec<PollerSpec> {
    let mut sqs_queues: BTreeMap<String, (String, BTreeMap<String, String>)> = BTreeMap::new();
    let mut pubsub_subs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut cf_queues: BTreeMap<(String, String, String), BTreeMap<String, String>> =
        BTreeMap::new();

    let mut names: Vec<&String> = cfg.buckets.keys().collect();
    names.sort();
    for name in names {
        let bucket = &cfg.buckets[name];
        let Some(material) = materials.get(name) else {
            continue;
        };
        let underlying = bucket.underlying(name);
        match bucket {
            BucketConfig::S3(value) => {
                if let Some(notification) = &value.notifications {
                    sqs_queues
                        .entry(notification.sqs_queue_url.clone())
                        .or_insert_with(|| (value.region.clone(), BTreeMap::new()))
                        .1
                        .insert(underlying, name.clone());
                }
            }
            BucketConfig::Gcs(value) => {
                if let Some(notification) = &value.notifications {
                    pubsub_subs
                        .entry(notification.pubsub_subscription.clone())
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            BucketConfig::R2(value) => {
                if let (Some(notification), Some(account_id), Some(api_token)) = (
                    &value.notifications,
                    material.value("account_id"),
                    material.value("notifications.api_token"),
                ) {
                    cf_queues
                        .entry((
                            account_id.to_string(),
                            notification.queue_id.clone(),
                            api_token.to_string(),
                        ))
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            BucketConfig::Local(_) => {}
        }
    }

    let served = |reverse: &BTreeMap<String, String>| {
        let mut buckets: Vec<String> = reverse.values().cloned().collect();
        buckets.sort();
        buckets
    };
    let reverse_key = |reverse: &BTreeMap<String, String>| {
        reverse
            .iter()
            .map(|(u, n)| format!("{u}={n}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut specs = Vec::new();
    for (queue_url, (region, reverse)) in sqs_queues {
        specs.push(PollerSpec {
            key: format!("sqs|{queue_url}|{region}|{}", reverse_key(&reverse)),
            buckets: served(&reverse),
            source: PollerSource::Sqs {
                queue_url,
                region,
                reverse,
            },
        });
    }
    for (subscription, reverse) in pubsub_subs {
        specs.push(PollerSpec {
            key: format!("pubsub|{subscription}|{}", reverse_key(&reverse)),
            buckets: served(&reverse),
            source: PollerSource::Pubsub {
                subscription,
                reverse,
            },
        });
    }
    for ((account_id, queue_id, api_token), reverse) in cf_queues {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&account_id, &queue_id, &api_token).hash(&mut hasher);
        specs.push(PollerSpec {
            key: format!("cf|{:016x}|{}", hasher.finish(), reverse_key(&reverse)),
            buckets: served(&reverse),
            source: PollerSource::CfQueue {
                account_id,
                queue_id,
                api_token,
                reverse,
            },
        });
    }
    specs
}

/// Register the `storage` schema. A seed is an initial value only; once the
/// configuration exists, the configuration worker remains authoritative.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Storage",
        "description": "Object storage buckets across AWS S3, GCS, Cloudflare R2, and a native local filesystem backend.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed
        .map(|value| value.to_json())
        .unwrap_or_else(|| WorkerConfig::default().to_json());
    ensure_configuration(iii, payload).await
}

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        trigger_configuration_with_retry(iii, function, payload)
    })
    .await
}

/// Read the live storage configuration (already env-expanded by the
/// configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json_isolating(&value)
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Read the assigned storage entry, preserving failed lookups instead of selecting defaults.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

pub fn register_config_trigger(
    iii: &IIIClient,
    state: AppState,
    runtime: Arc<StorageRuntime>,
) -> Result<(), Error> {
    let state_for_handler = state.clone();
    let runtime_for_handler = runtime.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let state = state_for_handler.clone();
            let runtime = runtime_for_handler.clone();
            async move {
                let result = runtime.reload(&state).await;
                match &result {
                    Ok(()) => tracing::info!("storage configuration reloaded without restart"),
                    Err(error) => tracing::error!(
                        error = %error,
                        "storage configuration reload failed; keeping previous runtime"
                    ),
                }
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse {
                    ok: result.is_ok(),
                })
            }
        })
        .description(
            "Internal: apply the authoritative storage configuration without restarting the worker.",
        )
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

/// Advisory `secrets::changed` delivery (never carries a value).
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnSecretChangeEvent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    /// `created` | `rotated` | `deleted` | `access_changed` (advisory).
    #[serde(default)]
    #[allow(dead_code)]
    pub action: Option<String>,
}

/// Register the internal `secrets::changed` handler and bind it. A rotated,
/// deleted or re-shared secret that a bucket references is re-read, and
/// only the buckets whose resolved values changed are rebuilt. The payload
/// is advisory (it never carries a value) and anything unrelated is
/// ignored, so a direct call can at most cause a re-read.
pub fn register_secret_trigger(iii: &IIIClient, state: AppState, runtime: Arc<StorageRuntime>) {
    iii.register_function(
        SECRET_FN_ID,
        RegisterFunction::new_async(move |event: OnSecretChangeEvent| {
            let state = state.clone();
            let runtime = runtime.clone();
            async move {
                let payload = json!({ "name": event.name, "ref": event.reference });
                let Some(reference) = crate::secrets::event_reference(&payload) else {
                    return Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse {
                        ok: false,
                    });
                };
                runtime.on_secret_changed(&state, &reference).await;
                Ok(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: re-read a rotated or revoked secret:// / env:// value and rebuild the \
             buckets that reference it.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    // `secrets::changed` belongs to the secrets worker: when it is absent the
    // engine parks this binding and activates it once the type registers.
    if let Err(error) = iii.register_trigger(RegisterTriggerInput::new(
        crate::secrets::CHANGED_TRIGGER_TYPE.to_string(),
        SECRET_FN_ID.to_string(),
        json!({}),
    )) {
        tracing::warn!(error = %error, "binding secrets::changed failed; secret references refresh on their TTL only");
    }
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_error = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error.to_string();
                if matches!(&error, iii_sdk::errors::Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_error);
                }
                if attempt < CONFIG_RETRIES {
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_error,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_error}"
    ))
}

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
fn is_not_found(error: &str) -> bool {
    // Anchor on the SDK's own rendering instead of scanning the whole string:
    // a remote failure prints as `remote error ({code}): {message}`, and this
    // worker wraps a retried get as
    // `configuration::get failed after CONFIG_RETRIES attempts: {err}`. Peel
    // exactly that one wrapper (never a foreign one or a different attempt
    // count) and then require the NOT_FOUND envelope at the very start, so a
    // NOT_FOUND code buried in an unrelated message, a nested envelope, or a
    // different wrapper stays a real failure and propagates.
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(CONFIG_RETRIES == 3);
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "NOT_FOUND"
        || raw == "remote error (NOT_FOUND):"
        || raw.starts_with("remote error (NOT_FOUND): ")
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/config-client/tests/support/is_not_found_cases.rs"
    ));

    /// The missing-entry classifier only seeds on the configuration worker's
    /// standalone `NOT_FOUND` envelope; every unrelated failure or compound
    /// code propagates instead of clobbering a stored value with a default.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }

    use super::*;
    use crate::error::StorageError;
    use crate::triggers::registry::TriggerRegistry;
    use iii_sdk::IIIClient;
    use tokio::sync::RwLock;

    #[test]
    fn live_configuration_accepts_zero_buckets() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({})).unwrap();
        assert!(cfg.buckets.is_empty());
    }

    #[test]
    fn wired_bucket_projection_tracks_live_sources() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({
            "buckets": {
                "local": { "provider": "local" },
                "cold": { "provider": "s3", "region": "us-east-1" },
                "events": {
                    "provider": "s3",
                    "region": "us-east-1",
                    "notifications": { "sqs_queue_url": "https://sqs.test/q" }
                },
                "broken": {
                    "provider": "s3",
                    "region": "us-east-1",
                    "notifications": { "sqs_queue_url": "https://sqs.test/q" }
                },
                "muted": {
                    "provider": "s3",
                    "region": "us-east-1",
                    "notifications": { "sqs_queue_url": "https://sqs.test/other" }
                }
            }
        }))
        .unwrap();
        let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();
        for name in ["local", "cold", "events", "muted"] {
            backends.insert(
                name.into(),
                Arc::new(crate::backend::mock::MockBackend::default()),
            );
        }
        let mut notification_errors = HashMap::new();
        notification_errors.insert("muted".to_string(), "auth".to_string());
        let wired = wired_buckets(&cfg, &backends, &notification_errors);
        assert!(wired.contains("local"));
        assert!(wired.contains("events"));
        assert!(!wired.contains("cold"), "no notifications source");
        assert!(!wired.contains("broken"), "no backend");
        assert!(!wired.contains("muted"), "its source failed");
    }

    #[test]
    fn poller_specs_group_by_source_and_hide_the_cloudflare_token() {
        let cfg = WorkerConfig::from_json(&serde_json::json!({
            "buckets": {
                "a": { "provider": "s3", "region": "eu-west-1", "bucket": "ua",
                       "notifications": { "sqs_queue_url": "https://sqs.test/q" } },
                "b": { "provider": "s3", "region": "us-east-1",
                       "notifications": { "sqs_queue_url": "https://sqs.test/q" } },
                "r": { "provider": "r2", "account_id": "acc", "access_key_id": "k",
                       "secret_access_key": "s", "notifications": { "queue_id": "q1", "api_token": "secret://CF" } },
                "down": { "provider": "s3", "region": "us-east-1",
                          "notifications": { "sqs_queue_url": "https://sqs.test/z" } }
            }
        }))
        .unwrap();
        let mut materials = HashMap::new();
        for name in ["a", "b"] {
            materials.insert(
                name.to_string(),
                Material::literal(&cfg.buckets[name]).unwrap(),
            );
        }
        materials.insert(
            "r".to_string(),
            Material::resolve(&cfg.buckets["r"], |_| {
                Some(Ok(crate::secrets::SecretValue::new(
                    "cf-token-value".into(),
                )))
            })
            .unwrap(),
        );
        let specs = poller_specs(&cfg, &materials);
        assert_eq!(
            specs.len(),
            2,
            "one SQS queue, one CF queue; `down` did not build"
        );
        let sqs = specs.iter().find(|s| s.key.starts_with("sqs|")).unwrap();
        assert_eq!(sqs.buckets, vec!["a".to_string(), "b".to_string()]);
        assert!(
            sqs.key.contains("eu-west-1"),
            "the first bucket's region: {}",
            sqs.key
        );
        let cf = specs.iter().find(|s| s.key.starts_with("cf|")).unwrap();
        assert!(!cf.key.contains("cf-token-value"), "{}", cf.key);
        assert!(
            matches!(&cf.source, PollerSource::CfQueue { api_token, .. } if api_token == "cf-token-value")
        );
        // Same inputs, same keys: the running pollers are kept.
        let again = poller_specs(&cfg, &materials);
        assert_eq!(
            specs.iter().map(|s| &s.key).collect::<Vec<_>>(),
            again.iter().map(|s| &s.key).collect::<Vec<_>>()
        );
    }

    fn test_runtime(state: &AppState) -> Arc<StorageRuntime> {
        StorageRuntime::new(
            Arc::new(EngineDispatcher::new(
                IIIClient::new("ws://127.0.0.1:1"),
                Arc::new(TriggerRegistry::new()),
            )),
            state,
            Arc::new(RwLock::new(HashSet::new())),
        )
    }

    #[tokio::test]
    async fn a_failing_bucket_is_isolated_and_reported() {
        let data_dir = tempfile::tempdir().unwrap();
        let fake = crate::secrets::testing::FakeSecrets::default();
        let state = AppState::with_secrets(HashMap::new(), Arc::new(fake.cache()));
        let runtime = test_runtime(&state);
        let cfg = WorkerConfig::from_json_isolating(&serde_json::json!({
            "providers": { "local": { "data_dir": data_dir.path().to_string_lossy() } },
            "buckets": {
                "ok": { "provider": "local" },
                "nosecret": { "provider": "s3", "region": "us-east-1", "access_key_id": "a",
                              "secret_access_key": "secret://MISSING" },
                "badgcs": { "provider": "gcs", "credentials_json": "{not json" },
                "Bad Name": { "provider": "local" },
                "noregion": { "provider": "s3" }
            }
        }))
        .unwrap();
        runtime.apply_config(&state, cfg).await.unwrap();
        assert!(state.backend("ok").await.is_ok());
        let catalog = state.catalog_snapshot().await;
        assert_eq!(catalog.errors["nosecret"].stage, Stage::Secrets);
        assert_eq!(catalog.errors["badgcs"].stage, Stage::Auth);
        assert_eq!(catalog.errors["Bad Name"].stage, Stage::Config);
        assert_eq!(catalog.errors["noregion"].stage, Stage::Config);
        for name in ["nosecret", "badgcs", "Bad Name", "noregion"] {
            let err = state.backend(name).await.err().unwrap().to_wire_string();
            assert!(err.contains("BUCKET_UNAVAILABLE"), "{name}: {err}");
        }
        assert!(matches!(
            state.backend("never-configured").await,
            Err(StorageError::UnknownBucket { .. })
        ));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn a_changed_secret_rebuilds_only_the_buckets_that_use_it() {
        let fake = crate::secrets::testing::FakeSecrets::default();
        fake.set("secret://A", Ok("a1"));
        fake.set("secret://B", Ok("b1"));
        let state = AppState::with_secrets(HashMap::new(), Arc::new(fake.cache()));
        let runtime = test_runtime(&state);
        let cfg = WorkerConfig::from_json_isolating(&serde_json::json!({
            "buckets": {
                "a": { "provider": "s3", "region": "us-east-1", "access_key_id": "k", "secret_access_key": "secret://A" },
                "b": { "provider": "s3", "region": "us-east-1", "access_key_id": "k", "secret_access_key": "secret://B" }
            }
        }))
        .unwrap();
        runtime.apply_config(&state, cfg).await.unwrap();
        let (a1, b1) = (
            state.backend("a").await.unwrap(),
            state.backend("b").await.unwrap(),
        );
        // An unrelated secret: forgotten, nothing rebuilt.
        runtime.on_secret_changed(&state, "secret://OTHER").await;
        assert!(Arc::ptr_eq(&a1, &state.backend("a").await.unwrap()));
        // Rotated: only `a` is rebuilt.
        fake.set("secret://A", Ok("a2"));
        runtime.on_secret_changed(&state, "secret://A").await;
        assert!(!Arc::ptr_eq(&a1, &state.backend("a").await.unwrap()));
        assert!(Arc::ptr_eq(&b1, &state.backend("b").await.unwrap()));
        // Deleted: `b` reports it; re-created: `b` comes back.
        fake.set("secret://B", Err(crate::secrets::SecretError::NotFound));
        runtime.on_secret_changed(&state, "secret://B").await;
        assert!(state.backend("b").await.is_err());
        assert_eq!(
            state.catalog_snapshot().await.errors["b"].stage,
            Stage::Secrets
        );
        fake.set("secret://B", Ok("b2"));
        runtime.on_secret_changed(&state, "secret://B").await;
        assert!(state.backend("b").await.is_ok());
        assert!(state.catalog_snapshot().await.errors.is_empty());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn runtime_applies_bucket_add_remove_and_listener_changes_live() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::new(HashMap::new());
        let wired = Arc::new(RwLock::new(HashSet::new()));
        let dispatcher = Arc::new(EngineDispatcher::new(
            IIIClient::new("ws://127.0.0.1:1"),
            Arc::new(TriggerRegistry::new()),
        ));
        let runtime = StorageRuntime::new(dispatcher, &state, wired.clone());
        let configured = WorkerConfig::from_json(&serde_json::json!({
            "providers": {
                "local": {
                    "data_dir": data_dir.path().to_string_lossy(),
                    "http": { "bind_address": "127.0.0.1:0" }
                }
            },
            "buckets": { "scratch": { "provider": "local" } }
        }))
        .unwrap();
        runtime.apply_config(&state, configured).await.unwrap();
        assert_eq!(state.backend("scratch").await.unwrap().provider(), "local");
        assert!(wired.read().await.contains("scratch"));
        assert!(runtime.local.current_addr().await.is_some());

        runtime
            .apply_config(
                &state,
                WorkerConfig::from_json(&serde_json::json!({})).unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(
            state.backend("scratch").await,
            Err(StorageError::UnknownBucket { .. })
        ));
        assert!(wired.read().await.is_empty());
        assert!(runtime.local.current_addr().await.is_none());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn failed_listener_rebind_preserves_the_published_runtime() {
        let data_dir = tempfile::tempdir().unwrap();
        let state = AppState::new(HashMap::new());
        let wired = Arc::new(RwLock::new(HashSet::new()));
        let dispatcher = Arc::new(EngineDispatcher::new(
            IIIClient::new("ws://127.0.0.1:1"),
            Arc::new(TriggerRegistry::new()),
        ));
        let runtime = StorageRuntime::new(dispatcher, &state, wired);
        let initial = WorkerConfig::from_json(&serde_json::json!({
            "providers": {
                "local": {
                    "data_dir": data_dir.path().to_string_lossy(),
                    "http": { "bind_address": "127.0.0.1:0" }
                }
            },
            "buckets": { "scratch": { "provider": "local" } }
        }))
        .unwrap();
        runtime.apply_config(&state, initial).await.unwrap();
        let original_addr = runtime.local.current_addr().await;

        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_addr = occupied.local_addr().unwrap();
        let rejected = WorkerConfig::from_json(&serde_json::json!({
            "providers": {
                "local": {
                    "data_dir": data_dir.path().to_string_lossy(),
                    "http": { "bind_address": occupied_addr.to_string() }
                }
            },
            "buckets": { "replacement": { "provider": "local" } }
        }))
        .unwrap();
        assert!(runtime.apply_config(&state, rejected).await.is_err());
        assert!(state.backend("scratch").await.is_ok());
        assert!(matches!(
            state.backend("replacement").await,
            Err(StorageError::UnknownBucket { .. })
        ));
        assert_eq!(runtime.local.current_addr().await, original_addr);
        runtime.shutdown().await;
    }
}
