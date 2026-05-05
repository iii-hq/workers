//! iii-state-backed `SessionStore`.
//!
//! Storage layout (scope-per-session for bounded scan cost):
//!
//! - Scope `session_tree:<session_id>`, key `<entry_id>`, value `SessionEntry`
//! - Scope `session_tree_meta`, key `<session_id>`, value `SessionMeta`
//!
//! `state::list` returns values without keys, so each entry's `id` field is
//! used to recover ordering when loading entries (Task E12).

use std::sync::Arc;

use iii_sdk::TriggerRequest;
use serde_json::json;

use crate::{
    io::IIITrigger, SessionEntry, SessionError, SessionMeta, SessionStore,
};

const META_SCOPE: &str = "session_tree_meta";

fn entries_scope(session_id: &str) -> String {
    format!("session_tree:{session_id}")
}

fn current_timestamp_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `SessionStore` impl that persists via `iii-state`.
pub struct IiiStateSessionStore {
    iii: Arc<dyn IIITrigger>,
}

impl IiiStateSessionStore {
    pub fn new(iii: Arc<dyn IIITrigger>) -> Self {
        Self { iii }
    }
}

#[async_trait::async_trait]
impl SessionStore for IiiStateSessionStore {
    async fn create(&self, meta: SessionMeta) -> Result<(), SessionError> {
        let session_id = meta.session_id.clone();
        let value = serde_json::to_value(&meta)
            .map_err(|e| SessionError::Storage(format!("serialize SessionMeta: {e}")))?;
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({
                    "scope": META_SCOPE,
                    "key": session_id,
                    "value": value,
                }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| SessionError::Storage(format!("state::set meta: {e}")))?;
        Ok(())
    }

    async fn append(&self, session_id: &str, entry: SessionEntry) -> Result<(), SessionError> {
        let entry_id = entry.id().to_string();
        let entry_value = serde_json::to_value(&entry)
            .map_err(|e| SessionError::Storage(format!("serialize SessionEntry: {e}")))?;
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({
                    "scope": entries_scope(session_id),
                    "key": entry_id,
                    "value": entry_value,
                }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| SessionError::Storage(format!("state::set entry: {e}")))?;

        // Non-fatal: refresh updated_at on the session's meta. Failures here log
        // a warning but do not roll back the entry write.
        if let Err(e) = self.refresh_meta_updated_at(session_id).await {
            log::warn!(
                target: "session_tree::store_iii_state",
                "meta updated_at refresh failed for session {session_id}: {e}; entry persisted but meta is stale"
            );
        }

        Ok(())
    }

    async fn load_entries(&self, _session_id: &str) -> Result<Vec<SessionEntry>, SessionError> {
        unimplemented!("Task E12 implements load_entries")
    }

    async fn load_meta(&self, _session_id: &str) -> Result<SessionMeta, SessionError> {
        unimplemented!("Task E13 implements load_meta")
    }

    async fn list(&self) -> Result<Vec<SessionMeta>, SessionError> {
        unimplemented!("Task E13 implements list")
    }
}

impl IiiStateSessionStore {
    async fn refresh_meta_updated_at(&self, session_id: &str) -> Result<(), SessionError> {
        let meta_value = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": META_SCOPE, "key": session_id }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| SessionError::Storage(format!("state::get meta: {e}")))?;
        if meta_value.is_null() {
            return Ok(()); // No meta yet (e.g., create wasn't called); nothing to refresh.
        }
        let mut meta: SessionMeta = serde_json::from_value(meta_value)
            .map_err(|e| SessionError::Storage(format!("deserialize SessionMeta: {e}")))?;
        meta.updated_at = current_timestamp_ms();
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({
                    "scope": META_SCOPE,
                    "key": session_id,
                    "value": serde_json::to_value(&meta)
                        .map_err(|e| SessionError::Storage(format!("serialize SessionMeta: {e}")))?,
                }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| SessionError::Storage(format!("state::set meta: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use async_trait::async_trait;
    use iii_sdk::IIIError;
    use serde_json::Value;

    struct MockTrigger {
        responses: Mutex<Vec<Result<Value, IIIError>>>,
        calls: Mutex<Vec<TriggerRequest>>,
    }

    impl MockTrigger {
        fn new(responses: Vec<Result<Value, IIIError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl IIITrigger for MockTrigger {
        async fn trigger(&self, request: TriggerRequest) -> Result<Value, IIIError> {
            self.calls.lock().unwrap().push(request);
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn sample_meta(id: &str) -> SessionMeta {
        SessionMeta {
            session_id: id.to_string(),
            display_name: Some(format!("session {id}")),
            created_at: 1_000,
            updated_at: 1_000,
            cwd: None,
            branch_count: 1,
        }
    }

    #[tokio::test]
    async fn create_writes_meta_to_state() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![Ok(Value::Null)]));
        let store = IiiStateSessionStore::new(mock.clone());

        store.create(sample_meta("s1")).await.map_err(|e| anyhow::anyhow!("{e}"))?;

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "state::set");
        assert_eq!(calls[0].payload["scope"], META_SCOPE);
        assert_eq!(calls[0].payload["key"], "s1");
        assert_eq!(calls[0].payload["value"]["session_id"], "s1");
        Ok(())
    }

    #[tokio::test]
    async fn create_returns_storage_err_on_trigger_failure() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler("boom".into()))]));
        let store = IiiStateSessionStore::new(mock);
        let result = store.create(sample_meta("s1")).await;
        assert!(matches!(result, Err(SessionError::Storage(_))));
    }

    #[test]
    fn entries_scope_namespaces_per_session() {
        assert_eq!(entries_scope("abc"), "session_tree:abc");
    }

    fn sample_entry(eid: &str) -> SessionEntry {
        SessionEntry::CustomMessage {
            id: eid.to_string(),
            parent_id: None,
            custom_type: "test".into(),
            content: serde_json::json!({"text": "hello"}),
            display: None,
            details: serde_json::Value::Null,
            timestamp: 1_500,
        }
    }

    #[tokio::test]
    async fn append_writes_entry_then_refreshes_meta() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![
            Ok(Value::Null),                                                  // 1. entry write
            Ok(serde_json::to_value(sample_meta("s1"))?),                    // 2. load meta
            Ok(Value::Null),                                                  // 3. write refreshed meta
        ]));
        let store = IiiStateSessionStore::new(mock.clone());
        store
            .append("s1", sample_entry("e1"))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);

        // Call 1: entry write
        assert_eq!(calls[0].function_id, "state::set");
        assert_eq!(calls[0].payload["scope"], "session_tree:s1");
        assert_eq!(calls[0].payload["key"], "e1");

        // Call 2: load meta
        assert_eq!(calls[1].function_id, "state::get");
        assert_eq!(calls[1].payload["scope"], META_SCOPE);
        assert_eq!(calls[1].payload["key"], "s1");

        // Call 3: write refreshed meta with updated_at bumped
        assert_eq!(calls[2].function_id, "state::set");
        assert_eq!(calls[2].payload["scope"], META_SCOPE);
        assert_eq!(calls[2].payload["key"], "s1");
        let new_updated_at = calls[2].payload["value"]["updated_at"].as_i64().unwrap();
        assert!(new_updated_at >= 1_000, "updated_at should be at least the original 1_000");
        Ok(())
    }

    #[tokio::test]
    async fn append_succeeds_when_meta_load_fails() -> anyhow::Result<()> {
        // Entry write succeeds; meta load fails. append() returns Ok (per spec)
        // and only the entry write call should have been issued before the
        // graceful early-return.
        let mock = Arc::new(MockTrigger::new(vec![
            Ok(Value::Null),                                              // entry write
            Err(IIIError::Handler("meta load boom".into())),              // meta load
        ]));
        let store = IiiStateSessionStore::new(mock.clone());
        let result = store.append("s1", sample_entry("e1")).await;
        assert!(
            result.is_ok(),
            "append should succeed even if meta refresh fails; got {:?}",
            result,
        );
        let calls = mock.calls.lock().unwrap();
        // Two calls: entry write + meta load attempt; no meta write.
        assert_eq!(calls.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn append_succeeds_when_meta_does_not_exist() -> anyhow::Result<()> {
        // Entry write succeeds; meta load returns null (no meta yet). append
        // returns Ok and skips the meta refresh write entirely.
        let mock = Arc::new(MockTrigger::new(vec![
            Ok(Value::Null), // entry write
            Ok(Value::Null), // load meta returns null
        ]));
        let store = IiiStateSessionStore::new(mock.clone());
        let result = store.append("s1", sample_entry("e1")).await;
        assert!(result.is_ok());
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 2); // entry write + meta load; no meta write
        Ok(())
    }

    #[tokio::test]
    async fn append_returns_err_when_entry_write_fails() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler(
            "entry boom".into(),
        ))]));
        let store = IiiStateSessionStore::new(mock);
        let result = store.append("s1", sample_entry("e1")).await;
        assert!(matches!(result, Err(SessionError::Storage(_))));
    }
}
