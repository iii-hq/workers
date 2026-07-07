//! Worker boot and shutdown wiring.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

use crate::adapter::SwappableAdapter;
use crate::adapters::builtin::BuiltinAdapter;
use crate::config::QueueConfig;
use crate::store::{FileStore, InMemoryStore, QueueStore};
use crate::trigger::{IiiInvoker, QueueTriggerHandler, SubscriberSpec};
use crate::TRIGGER_TYPE;

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
pub const BUILTIN_III_QUEUE_WORKER_ID: &str = "iii-queue";

pub type ConfigCell = Arc<RwLock<Arc<QueueConfig>>>;
pub type ApplyLock = Arc<Mutex<()>>;

pub struct BootHandle {
    pub adapter: Arc<SwappableAdapter>,
    pub trigger_handler: QueueTriggerHandler,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    pub async fn shutdown(self) {
        self.trigger_handler.shutdown().await;
    }
}

pub async fn start(iii: Arc<IIIClient>, config: QueueConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_iii_queue(&iii).await?;

    let store = build_store(&config).await?;
    let invoker = Arc::new(IiiInvoker::new(iii.clone()));
    let adapter = Arc::new(SwappableAdapter::new(Arc::new(BuiltinAdapter::new(
        store, invoker,
    ))));
    let config = Arc::new(RwLock::new(Arc::new(config.normalized())));
    let apply_lock = Arc::new(Mutex::new(()));

    crate::functions::register_all(&iii, adapter.clone());

    let trigger_handler = QueueTriggerHandler::new(adapter.clone());
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
    })
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
        };
        let store = build_store(&config).await.unwrap();
        store.enqueue("demo", json!({"ok": true})).await.unwrap();
        assert!(dir.join("queue_store.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
