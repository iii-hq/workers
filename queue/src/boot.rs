//! Worker boot and shutdown wiring.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::adapter::{QueueAdapter, SwappableAdapter};
use crate::adapters::builtin::BuiltinAdapter;
#[cfg(feature = "rabbitmq")]
use crate::adapters::rabbitmq::RabbitMQAdapter;
use crate::adapters::redis::RedisAdapter;
use crate::config::{adapter_config_value, QueueConfig};
use crate::runtime::FunctionQueueRuntime;
use crate::store::{FileStore, InMemoryStore, QueueStore};
use crate::trigger::{IiiInvoker, Invoker, QueueTriggerHandler, SubscriberSpec};
use crate::TRIGGER_TYPE;

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
pub const BUILTIN_III_QUEUE_WORKER_ID: &str = "iii-queue";

pub type ConfigCell = Arc<RwLock<Arc<QueueConfig>>>;
pub type ApplyLock = Arc<RwLock<()>>;

pub struct BootHandle {
    pub adapter: Arc<SwappableAdapter>,
    pub trigger_handler: QueueTriggerHandler,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
    pub runtime: FunctionQueueRuntime,
}

impl BootHandle {
    pub async fn shutdown(self) {
        self.runtime.shutdown().await;
        self.trigger_handler.shutdown().await;
    }
}

pub async fn start(iii: Arc<IIIClient>, config: QueueConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_iii_queue(&iii).await?;

    let invoker: Arc<dyn Invoker> = Arc::new(IiiInvoker::new(iii.clone()));
    // No fallback to builtin on a bad config: an adapter that fails to build
    // at boot (e.g. redis/rabbitmq unreachable) must fail the boot itself,
    // matching the engine's `make_adapter` behavior for `iii-queue`.
    let built = build_adapter(&config, invoker.clone()).await?;
    let adapter = Arc::new(SwappableAdapter::new(built, adapter_identity_name(&config)));
    let config = Arc::new(RwLock::new(Arc::new(config.normalized())));
    let apply_lock = Arc::new(RwLock::new(()));

    let runtime = FunctionQueueRuntime::new(
        iii.clone(),
        adapter.clone(),
        invoker,
        config.clone(),
        apply_lock.clone(),
    );
    let trigger_handler = QueueTriggerHandler::new(adapter.clone());

    // Restore every authoritative queue before exposing queue::define. This
    // closes the startup race where a concurrent definition could otherwise
    // be replaced by a stale boot snapshot. Restored jobs wait for their
    // target function to appear, so starting before dependent workers is safe.
    runtime.start().await?;

    crate::functions::register_all(&iii, adapter.clone(), runtime.clone());
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            TRIGGER_TYPE,
            "Durable queue subscriber",
            trigger_handler.clone(),
        )
        .trigger_request_format::<SubscriberSpec>(),
    );

    Ok(BootHandle {
        adapter,
        trigger_handler,
        config,
        apply_lock,
        runtime,
    })
}

/// Adapter factory: resolves `config`'s effective adapter name to a live
/// [`QueueAdapter`]. This is the single dispatch point every adapter (both
/// the ones this worker implements and any future one) is wired in from --
/// `boot::start` uses it at boot and `configuration::swap_adapter` reuses it
/// for hot-swaps, so both paths apply the exact same "which adapter for this
/// config" logic.
///
/// Errors are never papered over with a fallback to `builtin`: a caller that
/// wants a different failure policy (e.g. the hot-swap path keeping the
/// previous adapter alive) decides that at the call site.
pub async fn build_adapter(
    config: &QueueConfig,
    invoker: Arc<dyn Invoker>,
) -> anyhow::Result<Arc<dyn QueueAdapter>> {
    let name = config.effective_adapter_name();
    match name {
        // Legacy worker aliases keep working: they select the builtin store
        // backend (`build_store` already dispatches `store_method` between
        // in-memory and file-backed).
        "builtin" | "in_memory" | "file_based" => {
            let store = build_store(config).await?;
            Ok(Arc::new(BuiltinAdapter::new(store, invoker)))
        }
        "redis" => {
            let adapter_cfg = adapter_config_value(config);
            let adapter = RedisAdapter::from_config(adapter_cfg.as_ref(), invoker).await?;
            Ok(Arc::new(adapter))
        }
        #[cfg(feature = "rabbitmq")]
        "rabbitmq" => {
            let adapter_cfg = adapter_config_value(config);
            let adapter = RabbitMQAdapter::from_config(adapter_cfg.as_ref(), invoker).await?;
            Ok(Arc::new(adapter))
        }
        #[cfg(not(feature = "rabbitmq"))]
        "rabbitmq" => {
            anyhow::bail!(
                "queue adapter 'rabbitmq' requires the standalone queue worker to be built \
                 with the `rabbitmq` feature enabled"
            )
        }
        #[cfg(feature = "test-adapters")]
        "memory" => Ok(Arc::new(crate::adapters::memory::MemoryAdapter::new(
            invoker,
        ))),
        other => anyhow::bail!(
            "queue adapter '{other}' is not implemented by the standalone queue worker yet"
        ),
    }
}

/// The transport identity behind `config`'s effective adapter, for
/// [`SwappableAdapter::new`]/[`SwappableAdapter::replace`]. Distinct from
/// `config.effective_adapter_name()`: the legacy `in_memory`/`file_based`
/// aliases both select the `builtin` transport, so they report as `"builtin"`
/// here too (matching the identity `build_adapter` actually constructs).
pub fn adapter_identity_name(config: &QueueConfig) -> &str {
    match config.effective_adapter_name() {
        "in_memory" | "file_based" => "builtin",
        other => other,
    }
}

pub async fn build_store(config: &QueueConfig) -> anyhow::Result<Arc<dyn QueueStore>> {
    let adapter_name = config.effective_adapter_name();
    if adapter_name != "builtin" && adapter_name != "file_based" && adapter_name != "in_memory" {
        anyhow::bail!(
            "queue adapter '{adapter_name}' is not implemented by the standalone queue worker yet"
        );
    }

    // An absent `adapter` / `adapter.config` means builtin defaults; Null does
    // not deserialize into the struct, so only parse an actual object.
    let builtin = match config
        .adapter
        .as_ref()
        .and_then(|adapter| adapter.config.clone())
    {
        Some(value) if !value.is_null() => serde_json::from_value::<BuiltinAdapterConfig>(value)?,
        _ => BuiltinAdapterConfig::default(),
    };

    let store_method = builtin.store_method.as_deref().unwrap_or(adapter_name);
    match store_method {
        "file_based" => {
            let path = builtin
                .file_path
                .unwrap_or_else(|| "queue_store_data".to_string());
            let save_interval_ms = builtin.save_interval_ms.unwrap_or(5000);
            Ok(Arc::new(FileStore::open(path, save_interval_ms).await?))
        }
        "builtin" | "in_memory" => Ok(Arc::new(InMemoryStore::new())),
        other => anyhow::bail!("unknown builtin queue store_method '{other}'"),
    }
}

async fn guard_against_builtin_iii_queue(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_queue_active(&workers_list) {
        anyhow::bail!(
            "cannot start the queue worker: the built-in iii-queue worker is active and owns the \
             'durable:subscriber' trigger type. Remove iii-queue from the engine config, then \
             start this worker."
        );
    }

    Ok(())
}

fn builtin_iii_queue_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|workers| workers.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_QUEUE_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_QUEUE_WORKER_ID)
            })
        })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuiltinAdapterConfig {
    #[serde(default)]
    store_method: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    save_interval_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdapterEntry;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn detects_builtin_iii_queue_by_id() {
        let workers_list = json!({
            "workers": [
                { "id": "iii-queue", "name": "queue builtin", "status": "connected" },
                { "id": "shell", "name": "shell", "status": "connected" },
            ]
        });
        assert!(builtin_iii_queue_active(&workers_list));
    }

    #[test]
    fn detects_builtin_iii_queue_by_name() {
        let workers_list = json!({
            "workers": [
                { "id": "worker-123", "name": "iii-queue", "status": "connected" },
            ]
        });
        assert!(builtin_iii_queue_active(&workers_list));
    }

    #[test]
    fn no_match_when_absent() {
        let workers_list = json!({
            "workers": [
                { "id": "shell", "name": "shell", "status": "connected" },
            ]
        });
        assert!(!builtin_iii_queue_active(&workers_list));
    }

    #[test]
    fn no_match_when_workers_empty() {
        assert!(!builtin_iii_queue_active(&json!({ "workers": [] })));
    }

    #[test]
    fn no_match_when_workers_missing() {
        assert!(!builtin_iii_queue_active(&json!({})));
    }

    #[tokio::test]
    async fn build_store_defaults_to_in_memory_without_adapter() {
        let config = QueueConfig::default();
        let store = build_store(&config).await.unwrap();
        store.enqueue("demo", json!({"ok": true})).await.unwrap();
    }

    #[tokio::test]
    async fn build_store_accepts_adapter_without_config() {
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        let store = build_store(&config).await.unwrap();
        store.enqueue("demo", json!({"ok": true})).await.unwrap();
    }

    #[tokio::test]
    async fn build_store_accepts_file_based_config() {
        let dir = std::env::temp_dir().join(format!("queue_boot_{}", Uuid::new_v4()));
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: Some(json!({
                    "store_method": "file_based",
                    "file_path": dir.to_string_lossy(),
                    "save_interval_ms": 5
                })),
            }),
            ..QueueConfig::default()
        };
        let store = build_store(&config).await.unwrap();
        store.enqueue("demo", json!({"ok": true})).await.unwrap();
        assert!(dir.join("queue_store.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    struct NoopInvoker;

    #[async_trait::async_trait]
    impl Invoker for NoopInvoker {
        async fn call(
            &self,
            _function_id: &str,
            _payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
    }

    fn noop_invoker() -> Arc<dyn Invoker> {
        Arc::new(NoopInvoker)
    }

    #[tokio::test]
    async fn build_adapter_defaults_to_builtin_without_config() {
        let adapter = build_adapter(&QueueConfig::default(), noop_invoker())
            .await
            .unwrap();
        adapter
            .enqueue("demo", json!({"ok": true}), None, None)
            .await;
    }

    #[tokio::test]
    async fn build_adapter_accepts_legacy_in_memory_alias() {
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "in_memory".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        build_adapter(&config, noop_invoker()).await.unwrap();
    }

    #[tokio::test]
    async fn build_adapter_accepts_legacy_file_based_alias() {
        let dir = std::env::temp_dir().join(format!("queue_boot_adapter_{}", Uuid::new_v4()));
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "file_based".to_string(),
                config: Some(json!({
                    "file_path": dir.to_string_lossy(),
                    "save_interval_ms": 5
                })),
            }),
            ..QueueConfig::default()
        };
        build_adapter(&config, noop_invoker()).await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn build_adapter_rejects_unknown_adapter() {
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "sqs".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        let err = match build_adapter(&config, noop_invoker()).await {
            Ok(_) => panic!("expected 'sqs' to be rejected"),
            Err(err) => err,
        };
        assert_eq!(
            err.to_string(),
            "queue adapter 'sqs' is not implemented by the standalone queue worker yet"
        );
    }

    // NOTE: this test never actually runs under plain `cargo test`, even with
    // `--no-default-features`. The dev-dependency self-reference
    // (`iii-queue = { path = ".", features = ["test-adapters"] }` above)
    // doesn't set `default-features = false`, so Cargo's feature unification
    // pulls the default `rabbitmq` feature back in for the whole test build
    // regardless of what the outer `cargo test` invocation requested. The
    // `#[cfg(not(feature = "rabbitmq"))]` guard is therefore always false in
    // practice; this only compiles/runs if something changes the dev-dep to
    // disable default features. Same gap as the engine's equivalent test
    // (`engine/Cargo.toml:168-171`) — left unfixed here to stay 1:1 with it.
    #[cfg(not(feature = "rabbitmq"))]
    #[tokio::test]
    async fn build_adapter_rabbitmq_without_feature_mentions_the_feature() {
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "rabbitmq".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        let err = match build_adapter(&config, noop_invoker()).await {
            Ok(_) => panic!("expected 'rabbitmq' to be rejected without the feature"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("rabbitmq"),
            "error should mention the missing feature: {err}"
        );
        assert!(
            err.to_string().contains("feature"),
            "error should mention the missing feature: {err}"
        );
    }

    #[cfg(feature = "test-adapters")]
    #[tokio::test]
    async fn build_adapter_selects_memory_test_adapter() {
        let config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "memory".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        let adapter = build_adapter(&config, noop_invoker()).await.unwrap();
        adapter
            .enqueue("demo", json!({"ok": true}), None, None)
            .await;
    }

    #[test]
    fn adapter_identity_name_maps_legacy_aliases_to_builtin() {
        let in_memory = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "in_memory".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        let file_based = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "file_based".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_identity_name(&in_memory), "builtin");
        assert_eq!(adapter_identity_name(&file_based), "builtin");
        assert_eq!(adapter_identity_name(&QueueConfig::default()), "builtin");
    }

    #[test]
    fn adapter_identity_name_passes_through_other_names() {
        let redis = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_identity_name(&redis), "redis");
    }
}
