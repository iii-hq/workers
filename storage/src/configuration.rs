//! Integration with the `configuration` worker — registration, authoritative
//! reads, and transactional live application of every storage setting.

use crate::backend::factory::{self, LocalBackendCtx};
use crate::backend::local::LocalRuntime;
use crate::backend::Backend;
use crate::config::{BucketConfig, WorkerConfig};
use crate::handlers::AppState;
use crate::triggers::dispatcher::EngineDispatcher;
use crate::triggers::handler::WiredBuckets;
use crate::triggers::pollers::{cf_queue, pubsub, sqs};
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
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
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// The live resources that configuration can replace: native-local service and
/// listener, cloud notification pollers, and the trigger registration wiring
/// set. `apply_lock` serializes the complete fetch → prepare → publish flow so
/// rapid edits cannot land out of order.
pub struct StorageRuntime {
    local: Arc<LocalRuntime>,
    dispatcher: Arc<EngineDispatcher>,
    wired_buckets: WiredBuckets,
    pollers: Mutex<Vec<JoinHandle<()>>>,
    apply_lock: Mutex<()>,
}

impl StorageRuntime {
    pub fn new(
        dispatcher: Arc<EngineDispatcher>,
        state: &AppState,
        wired_buckets: WiredBuckets,
    ) -> Arc<Self> {
        Arc::new(Self {
            local: Arc::new(LocalRuntime::new(
                dispatcher.clone(),
                state.reconfigure_gate.clone(),
            )),
            dispatcher,
            wired_buckets,
            pollers: Mutex::new(Vec::new()),
            apply_lock: Mutex::new(()),
        })
    }

    /// Apply a known-good snapshot (used at boot and by unit tests). Every
    /// fallible prerequisite is resolved before the short publish phase.
    pub async fn apply_config(&self, state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
        let _apply = self.apply_lock.lock().await;
        self.prepare_and_publish(state, cfg).await
    }

    /// Re-fetch and apply the authoritative stored value while holding the
    /// serialization lock end-to-end. The trigger payload is intentionally not
    /// trusted because the function is discoverable on the bus.
    async fn reload_authoritative(&self, iii: &IIIClient, state: &AppState) -> Result<(), String> {
        let _apply = self.apply_lock.lock().await;
        let cfg = fetch_config(iii).await?;
        self.prepare_and_publish(state, cfg).await
    }

    async fn prepare_and_publish(&self, state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
        let needs_local = cfg
            .buckets
            .values()
            .any(|bucket| matches!(bucket, BucketConfig::Local(_)));
        let local_update = self
            .local
            .prepare_update(cfg.providers.local.as_ref(), needs_local)
            .await
            .map_err(|error| format!("preparing native local storage: {error}"))?;
        let new_backends = build_backends(&cfg, local_update.context()).await?;
        let prepared_pollers = prepare_pollers(&cfg).await?;
        let new_wired_buckets = configured_wired_buckets(&cfg);

        // Requests and signed HTTP handlers take the read side only while
        // selecting an Arc-backed generation. Holding the write side here makes
        // the multi-resource publication atomic from their perspective.
        let _publish = state.reconfigure_gate.write().await;
        self.local.commit(local_update).await;
        *state.backends.write().await = new_backends;
        *self.wired_buckets.write().await = new_wired_buckets;

        let mut running_pollers = self.pollers.lock().await;
        for handle in running_pollers.drain(..) {
            handle.abort();
        }
        *running_pollers = prepared_pollers
            .into_iter()
            .map(|poller| poller.spawn(self.dispatcher.clone()))
            .collect();
        drop(running_pollers);

        if let Some(addr) = self.local.current_addr().await {
            tracing::info!(address = %addr, "native local storage HTTP server ready");
        }
        let notification_sources = self.pollers.lock().await.len();
        tracing::info!(
            buckets = cfg.buckets.len(),
            notification_sources,
            "storage configuration applied live"
        );
        Ok(())
    }

    pub async fn shutdown(&self) {
        let _apply = self.apply_lock.lock().await;
        for handle in self.pollers.lock().await.drain(..) {
            handle.abort();
        }
        self.local.shutdown().await;
    }
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
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live storage configuration (already env-expanded by the
/// configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(error) if error.contains("NOT_FOUND") => Ok(None),
        Err(error) => Err(error),
    }
}

/// Build every configured backend against a prepared local generation. This is
/// intentionally separate from publication so a failed provider setup leaves
/// the currently-running map untouched.
pub async fn build_backends(
    cfg: &WorkerConfig,
    local_ctx: Option<&LocalBackendCtx>,
) -> Result<HashMap<String, Arc<dyn Backend>>, String> {
    let mut backends = HashMap::new();
    for (name, bucket_cfg) in &cfg.buckets {
        let backend = factory::build(name, bucket_cfg, &cfg.providers, local_ctx)
            .await
            .map_err(|error| format!("building backend `{name}`: {error}"))?;
        tracing::info!(bucket = %name, provider = backend.provider(), "backend ready");
        backends.insert(name.clone(), backend);
    }
    Ok(backends)
}

fn configured_wired_buckets(cfg: &WorkerConfig) -> HashSet<String> {
    cfg.buckets
        .iter()
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

async fn prepare_pollers(cfg: &WorkerConfig) -> Result<Vec<PreparedPoller>, String> {
    let mut sqs_queues: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut pubsub_subs: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cf_queues: HashMap<(String, String, String), HashMap<String, String>> = HashMap::new();
    let mut sqs_regions: HashMap<String, String> = HashMap::new();

    for (name, bucket) in &cfg.buckets {
        match bucket {
            BucketConfig::S3(value) => {
                if let Some(notification) = &value.notifications {
                    let underlying = value.bucket.clone().unwrap_or_else(|| name.clone());
                    sqs_queues
                        .entry(notification.sqs_queue_url.clone())
                        .or_default()
                        .insert(underlying, name.clone());
                    sqs_regions
                        .entry(notification.sqs_queue_url.clone())
                        .or_insert_with(|| value.region.clone());
                }
            }
            BucketConfig::Gcs(value) => {
                if let Some(notification) = &value.notifications {
                    let underlying = value.bucket.clone().unwrap_or_else(|| name.clone());
                    pubsub_subs
                        .entry(notification.pubsub_subscription.clone())
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            BucketConfig::R2(value) => {
                if let Some(notification) = &value.notifications {
                    let underlying = value.bucket.clone().unwrap_or_else(|| name.clone());
                    cf_queues
                        .entry((
                            value.account_id.clone(),
                            notification.queue_id.clone(),
                            notification.api_token.clone(),
                        ))
                        .or_default()
                        .insert(underlying, name.clone());
                }
            }
            BucketConfig::Local(_) => {}
        }
    }

    let mut prepared = Vec::new();
    for (queue_url, reverse) in sqs_queues {
        let region = sqs_regions
            .get(&queue_url)
            .cloned()
            .unwrap_or_else(|| "us-east-1".to_string());
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;
        prepared.push(PreparedPoller::Sqs(sqs::SqsPoller::new(
            aws_sdk_sqs::Client::new(&sdk_config),
            queue_url,
            Arc::new(reverse),
        )));
    }

    for (subscription, reverse) in pubsub_subs {
        let client_config = gcloud_pubsub::client::ClientConfig::default()
            .with_auth()
            .await
            .map_err(|error| format!("Pub/Sub auth for `{subscription}`: {error}"))?;
        let client = gcloud_pubsub::client::Client::new(client_config)
            .await
            .map_err(|error| format!("Pub/Sub client for `{subscription}`: {error}"))?;
        prepared.push(PreparedPoller::Pubsub(
            pubsub::PubsubPoller::new(subscription, Arc::new(reverse)),
            client,
        ));
    }

    for ((account_id, queue_id, api_token), reverse) in cf_queues {
        cf_queue::probe_auth(&account_id, &queue_id, &api_token)
            .await
            .map_err(|error| {
                format!(
                    "Cloudflare queue auth for `{queue_id}`: {}",
                    error.to_wire_string()
                )
            })?;
        prepared.push(PreparedPoller::CfQueue(cf_queue::CfQueuePoller::new(
            account_id,
            queue_id,
            api_token,
            Arc::new(reverse),
        )));
    }
    Ok(prepared)
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
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let state = state_for_handler.clone();
            let runtime = runtime_for_handler.clone();
            let engine = engine.clone();
            async move {
                let result = runtime.reload_authoritative(&engine, &state).await;
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
            "Internal: transactionally apply the authoritative storage configuration without restarting the worker.",
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

#[cfg(test)]
mod tests {
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
                }
            }
        }))
        .unwrap();
        let wired = configured_wired_buckets(&cfg);
        assert!(wired.contains("local"));
        assert!(wired.contains("events"));
        assert!(!wired.contains("cold"));
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
