//! iii-state-backed `CredentialStore`.
//!
//! Stores each credential under scope `auth_credentials`, key
//! `credential:<provider>`, value `<Credential>`.
//!
//! Failures from `state::*` triggers surface as `anyhow::Error` to callers,
//! consistent with the trait surface introduced in Task E1.

use std::sync::Arc;

use iii_sdk::TriggerRequest;
use serde_json::json;

use crate::{io::IIITrigger, Credential, CredentialStore};

const SCOPE: &str = "auth_credentials";

fn key_for(provider: &str) -> String {
    format!("credential:{provider}")
}

/// `CredentialStore` impl that persists via `iii-state`.
pub struct IiiStateCredentialStore {
    iii: Arc<dyn IIITrigger>,
}

impl IiiStateCredentialStore {
    pub fn new(iii: Arc<dyn IIITrigger>) -> Self {
        Self { iii }
    }
}

#[async_trait::async_trait]
impl CredentialStore for IiiStateCredentialStore {
    async fn get(&self, provider: &str) -> anyhow::Result<Option<Credential>> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": SCOPE, "key": key_for(provider) }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::get failed: {e}"))?;
        if resp.is_null() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(resp).map_err(|e| {
            anyhow::anyhow!("deserialize credential: {e}")
        })?))
    }

    async fn set(&self, _provider: &str, _credential: Credential) -> anyhow::Result<()> {
        unimplemented!("Task E4 implements set")
    }

    async fn clear(&self, _provider: &str) -> anyhow::Result<()> {
        unimplemented!("Task E4 implements clear")
    }

    async fn list(&self) -> anyhow::Result<Vec<(String, Credential)>> {
        unimplemented!("Task E5 implements list")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use async_trait::async_trait;
    use iii_sdk::IIIError;
    use serde_json::Value;

    /// Records `trigger` calls and returns canned responses in FIFO order.
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

    #[tokio::test]
    async fn get_returns_credential_on_hit() -> anyhow::Result<()> {
        let cred = Credential::ApiKey { key: "sk-test".into() };
        let mock = Arc::new(MockTrigger::new(vec![Ok(serde_json::to_value(&cred)?)]));
        let store = IiiStateCredentialStore::new(mock.clone());

        let result = store.get("anthropic").await?;
        assert_eq!(result, Some(cred));

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "state::get");
        assert_eq!(calls[0].payload["scope"], SCOPE);
        assert_eq!(calls[0].payload["key"], "credential:anthropic");
        Ok(())
    }

    #[tokio::test]
    async fn get_returns_none_on_null_response() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![Ok(Value::Null)]));
        let store = IiiStateCredentialStore::new(mock);
        assert_eq!(store.get("anthropic").await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn get_returns_err_on_trigger_failure() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler("boom".into()))]));
        let store = IiiStateCredentialStore::new(mock);
        assert!(store.get("anthropic").await.is_err());
    }

    #[test]
    fn key_for_uses_provider_prefix() {
        assert_eq!(key_for("anthropic"), "credential:anthropic");
    }
}
