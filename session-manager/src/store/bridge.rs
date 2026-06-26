//! Bridge backend: defer every raw storage operation to a **main**
//! session-manager running on another iii instance, via its internal
//! `session::store::*` protocol (served only by authoritative/fs-mode
//! instances — see `functions::store_protocol`).
//!
//! The bridged instance keeps all domain logic, per-session locks,
//! idempotency and revision rules; the main is pure durable storage.
//! Event propagation rides the same connection (see
//! `events::RemotePublisher` / `events::attach_bridge_relay`).

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use super::{SessionStore, StoreError};
use crate::functions::store_protocol;
use crate::types::{SessionEntry, SessionMeta};

pub struct BridgeStore {
    remote: Arc<IIIClient>,
    timeout_ms: u64,
}

impl BridgeStore {
    /// `remote` is a connection to the main instance's engine.
    pub fn new(remote: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { remote, timeout_ms }
    }

    async fn call(&self, function_id: &str, payload: Value) -> Result<Value, StoreError> {
        self.remote
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| StoreError(format!("bridge {function_id} failed: {e}")))
    }

    fn parse<T: serde::de::DeserializeOwned>(
        function_id: &str,
        value: Value,
    ) -> Result<T, StoreError> {
        serde_json::from_value(value)
            .map_err(|e| StoreError(format!("bridge {function_id} returned malformed data: {e}")))
    }
}

#[async_trait]
impl SessionStore for BridgeStore {
    async fn get_meta(&self, session_id: &str) -> Result<Option<SessionMeta>, StoreError> {
        let v = self
            .call(
                store_protocol::GET_META,
                json!({ "session_id": session_id }),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Self::parse(store_protocol::GET_META, v).map(Some)
    }

    async fn put_meta(&self, meta: &SessionMeta) -> Result<(), StoreError> {
        self.call(store_protocol::PUT_META, json!({ "meta": meta }))
            .await
            .map(|_| ())
    }

    async fn delete_meta(&self, session_id: &str) -> Result<(), StoreError> {
        self.call(
            store_protocol::DELETE_META,
            json!({ "session_id": session_id }),
        )
        .await
        .map(|_| ())
    }

    async fn list_metas(&self) -> Result<Vec<SessionMeta>, StoreError> {
        let v = self.call(store_protocol::LIST_METAS, json!({})).await?;
        let metas = v
            .get("metas")
            .cloned()
            .ok_or_else(|| StoreError("bridge list_metas response missing `metas`".into()))?;
        Self::parse(store_protocol::LIST_METAS, metas)
    }

    async fn get_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<Option<SessionEntry>, StoreError> {
        let v = self
            .call(
                store_protocol::GET_ENTRY,
                json!({ "session_id": session_id, "entry_id": entry_id }),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Self::parse(store_protocol::GET_ENTRY, v).map(Some)
    }

    async fn put_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        self.call(
            store_protocol::PUT_ENTRY,
            json!({ "session_id": session_id, "entry": entry }),
        )
        .await
        .map(|_| ())
    }

    async fn list_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, StoreError> {
        let v = self
            .call(
                store_protocol::LIST_ENTRIES,
                json!({ "session_id": session_id }),
            )
            .await?;
        let entries = v
            .get("entries")
            .cloned()
            .ok_or_else(|| StoreError("bridge list_entries response missing `entries`".into()))?;
        Self::parse(store_protocol::LIST_ENTRIES, entries)
    }

    async fn delete_entries(&self, session_id: &str) -> Result<(), StoreError> {
        self.call(
            store_protocol::DELETE_ENTRIES,
            json!({ "session_id": session_id }),
        )
        .await
        .map(|_| ())
    }

    async fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        let v = self
            .call(
                store_protocol::GET_ACTIVE_LEAF,
                json!({ "session_id": session_id }),
            )
            .await?;
        match v.get("entry_id") {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Null) | None => Ok(None),
            Some(other) => Err(StoreError(format!(
                "bridge get_active_leaf returned malformed entry_id: {other}"
            ))),
        }
    }

    async fn set_active_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), StoreError> {
        self.call(
            store_protocol::SET_ACTIVE_LEAF,
            json!({ "session_id": session_id, "entry_id": entry_id }),
        )
        .await
        .map(|_| ())
    }

    async fn delete_active_leaf(&self, session_id: &str) -> Result<(), StoreError> {
        self.call(
            store_protocol::DELETE_ACTIVE_LEAF,
            json!({ "session_id": session_id }),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_sdk::{register_worker, InitOptions};

    /// An unreachable main must surface as a storage error (which the
    /// service maps to `session/storage`) — never a panic or a hang.
    #[tokio::test]
    async fn unreachable_remote_maps_to_storage_error() {
        // Port 9 (discard) — nothing listens there.
        let remote = Arc::new(register_worker("ws://127.0.0.1:9", InitOptions::default()));
        let store = BridgeStore::new(remote.clone(), 300);
        let err = store.get_meta("s_x").await.unwrap_err();
        assert!(
            err.0.contains("session::store::get-meta"),
            "error should name the failing protocol call: {}",
            err.0
        );
        let session_err: crate::error::SessionError = err.into();
        assert_eq!(session_err.code(), "session/storage");
        remote.shutdown_async().await;
    }
}
