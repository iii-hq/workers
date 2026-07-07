//! Durable persistence behind a narrow trait so the land machine and the
//! lifecycle handlers are testable without an engine. Production dispatches
//! to the built-in `state::*` functions; tests use [`InMemoryStateStore`].
//!
//! iii-state has no compare-and-set, so correctness is single-instance:
//! every read-modify-write path holds the per-repo lock (`locks.rs`), the
//! same stance as the `workflow` worker.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::error::{codes, WError};
use crate::types::{LandJob, WorktreeRecord};

pub const SCOPE_RECORD: &str = "worktree";
pub const SCOPE_JOB: &str = "worktree_land_job";
pub const SCOPE_MARKER: &str = "worktree_land_active";

const DISPATCH_TIMEOUT_MS: u64 = 30_000;

#[async_trait]
pub trait StateStore: Send + Sync {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<Value>, WError>;
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), WError>;
    async fn delete(&self, scope: &str, key: &str) -> Result<(), WError>;
    async fn list(&self, scope: &str) -> Result<Vec<Value>, WError>;
}

/// Production store: dispatches to the engine's `state::*` functions.
pub struct IiiStateStore {
    iii: std::sync::Arc<IIIClient>,
}

impl IiiStateStore {
    pub fn new(iii: std::sync::Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl StateStore for IiiStateStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<Value>, WError> {
        let v = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: Some(DISPATCH_TIMEOUT_MS),
            })
            .await
            .map_err(|e| {
                WError::new(
                    codes::STATE_UNAVAILABLE,
                    format!("state::get {scope}/{key}: {e}"),
                )
            })?;
        if v.is_null() {
            Ok(None)
        } else {
            Ok(Some(v))
        }
    }

    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), WError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: Some(DISPATCH_TIMEOUT_MS),
            })
            .await
            .map(|_| ())
            .map_err(|e| {
                WError::new(
                    codes::STATE_UNAVAILABLE,
                    format!("state::set {scope}/{key}: {e}"),
                )
            })
    }

    async fn delete(&self, scope: &str, key: &str) -> Result<(), WError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::delete".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: Some(DISPATCH_TIMEOUT_MS),
            })
            .await
            .map(|_| ())
            .map_err(|e| {
                WError::new(
                    codes::STATE_UNAVAILABLE,
                    format!("state::delete {scope}/{key}: {e}"),
                )
            })
    }

    async fn list(&self, scope: &str) -> Result<Vec<Value>, WError> {
        let v = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope }),
                action: None,
                timeout_ms: Some(DISPATCH_TIMEOUT_MS),
            })
            .await
            .map_err(|e| {
                WError::new(
                    codes::STATE_UNAVAILABLE,
                    format!("state::list {scope}: {e}"),
                )
            })?;
        Ok(parse_value_list(&v))
    }
}

/// Tolerates the three shapes the state engine returns for a list:
/// a bare array, `{"values": [...]}` / `{"items": [...]}`, or a key-value map.
pub fn parse_value_list(v: &Value) -> Vec<Value> {
    if let Some(arr) = v.as_array() {
        arr.clone()
    } else if let Some(obj) = v.as_object() {
        if let Some(arr) = obj.get("values").and_then(|x| x.as_array()) {
            arr.clone()
        } else if let Some(arr) = obj.get("items").and_then(|x| x.as_array()) {
            arr.clone()
        } else {
            obj.values().cloned().collect()
        }
    } else {
        vec![]
    }
}

/// In-memory store for engine-free tests.
#[derive(Default)]
pub struct InMemoryStateStore {
    inner: Mutex<HashMap<(String, String), Value>>,
}

impl InMemoryStateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StateStore for InMemoryStateStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<Value>, WError> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(scope.to_string(), key.to_string()))
            .cloned())
    }

    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), WError> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((scope.to_string(), key.to_string()), value);
        Ok(())
    }

    async fn delete(&self, scope: &str, key: &str) -> Result<(), WError> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(scope.to_string(), key.to_string()));
        Ok(())
    }

    async fn list(&self, scope: &str) -> Result<Vec<Value>, WError> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|((s, _), _)| s == scope)
            .map(|(_, v)| v.clone())
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Typed helpers over the trait
// ---------------------------------------------------------------------------

pub async fn get_record(
    store: &dyn StateStore,
    worktree_id: &str,
) -> Result<Option<WorktreeRecord>, WError> {
    match store.get(SCOPE_RECORD, worktree_id).await? {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}

pub async fn put_record(store: &dyn StateStore, record: &WorktreeRecord) -> Result<(), WError> {
    store
        .set(
            SCOPE_RECORD,
            &record.worktree_id,
            serde_json::to_value(record)?,
        )
        .await
}

pub async fn delete_record(store: &dyn StateStore, worktree_id: &str) -> Result<(), WError> {
    store.delete(SCOPE_RECORD, worktree_id).await
}

/// All parseable records, in store order. For pure counts/filters where the
/// oldest-first ordering of [`list_records`] would be wasted work.
pub async fn collect_records(store: &dyn StateStore) -> Result<Vec<WorktreeRecord>, WError> {
    Ok(store
        .list(SCOPE_RECORD)
        .await?
        .into_iter()
        .filter_map(|v| {
            let id = v
                .get("worktree_id")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            match serde_json::from_value::<WorktreeRecord>(v) {
                Ok(record) => Some(record),
                Err(e) => {
                    tracing::warn!(
                        code = codes::STATE_CORRUPT,
                        worktree_id = ?id,
                        error = %e,
                        "skipping corrupt worktree record"
                    );
                    None
                }
            }
        })
        .collect())
}

pub async fn list_records(store: &dyn StateStore) -> Result<Vec<WorktreeRecord>, WError> {
    let mut records = collect_records(store).await?;
    records.sort_by_key(|r| r.created_at);
    Ok(records)
}

pub async fn get_job(store: &dyn StateStore, job_id: &str) -> Result<Option<LandJob>, WError> {
    match store.get(SCOPE_JOB, job_id).await? {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}

pub async fn put_job(store: &dyn StateStore, job: &LandJob) -> Result<(), WError> {
    store
        .set(SCOPE_JOB, &job.job_id, serde_json::to_value(job)?)
        .await
}

pub async fn delete_job(store: &dyn StateStore, job_id: &str) -> Result<(), WError> {
    store.delete(SCOPE_JOB, job_id).await
}

/// Active-land marker: worktree_id -> job_id, enforcing one live job per worktree.
pub async fn get_active_job_id(
    store: &dyn StateStore,
    worktree_id: &str,
) -> Result<Option<String>, WError> {
    Ok(store
        .get(SCOPE_MARKER, worktree_id)
        .await?
        .and_then(|v| v.as_str().map(str::to_string)))
}

pub async fn set_active_job_id(
    store: &dyn StateStore,
    worktree_id: &str,
    job_id: &str,
) -> Result<(), WError> {
    store.set(SCOPE_MARKER, worktree_id, json!(job_id)).await
}

pub async fn clear_active_job_id(store: &dyn StateStore, worktree_id: &str) -> Result<(), WError> {
    store.delete(SCOPE_MARKER, worktree_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_value_list_tolerates_all_shapes() {
        let rec = json!({ "a": 1 });
        assert_eq!(parse_value_list(&json!([rec.clone()])).len(), 1);
        assert_eq!(
            parse_value_list(&json!({ "values": [rec.clone()] })).len(),
            1
        );
        assert_eq!(
            parse_value_list(&json!({ "items": [rec.clone()] })).len(),
            1
        );
        assert_eq!(parse_value_list(&json!({ "k": rec })).len(), 1);
        assert!(parse_value_list(&json!(null)).is_empty());
    }

    #[tokio::test]
    async fn in_memory_store_round_trips() {
        let store = InMemoryStateStore::new();
        store.set("s", "k", json!({ "v": 1 })).await.unwrap();
        assert_eq!(store.get("s", "k").await.unwrap(), Some(json!({ "v": 1 })));
        assert_eq!(store.list("s").await.unwrap().len(), 1);
        assert!(store.list("other").await.unwrap().is_empty());
        store.delete("s", "k").await.unwrap();
        assert_eq!(store.get("s", "k").await.unwrap(), None);
    }
}
