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

#[allow(dead_code)] // used in E11-E12 once entry methods land
fn entries_scope(session_id: &str) -> String {
    format!("session_tree:{session_id}")
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

    async fn append(&self, _session_id: &str, _entry: SessionEntry) -> Result<(), SessionError> {
        unimplemented!("Task E11 implements append")
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
}
