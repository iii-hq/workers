//! iii-state-backed `CredentialStore`.
//!
//! Stores each credential under scope `auth_credentials`, key
//! `credential:<provider>`, value
//! `{ "provider": "<provider>", "credential": <Credential> }`.
//!
//! The provider name is embedded in the value so `list` can return
//! `Vec<(String, Credential)>` — `state::list` returns values without keys,
//! so we recover the provider name from the value itself.
//!
//! Failures from `state::*` triggers surface as `anyhow::Error` to callers,
//! consistent with the trait surface introduced in Task E1.

use std::sync::Arc;

use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

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
        let cred_value = resp
            .get("credential")
            .ok_or_else(|| {
                anyhow::anyhow!("malformed credential record (missing `credential` field)")
            })?
            .clone();
        Ok(Some(serde_json::from_value(cred_value).map_err(|e| {
            anyhow::anyhow!("deserialize credential: {e}")
        })?))
    }

    async fn set(&self, provider: &str, credential: Credential) -> anyhow::Result<()> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({
                    "scope": SCOPE,
                    "key": key_for(provider),
                    "value": json!({
                        "provider": provider,
                        "credential": serde_json::to_value(&credential)
                            .map_err(|e| anyhow::anyhow!("serialize credential: {e}"))?,
                    }),
                }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::set failed: {e}"))?;
        Ok(())
    }

    async fn clear(&self, provider: &str) -> anyhow::Result<()> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "state::delete".into(),
                payload: json!({ "scope": SCOPE, "key": key_for(provider) }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::delete failed: {e}"))?;
        Ok(())
    }

    async fn list(&self) -> anyhow::Result<Vec<(String, Credential)>> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": SCOPE }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("state::list failed: {e}"))?;
        let arr = resp
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("state::list returned non-array"))?;
        let mut out = Vec::with_capacity(arr.len());
        for value in arr {
            let provider = value
                .get("provider")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("malformed list entry: missing `provider`"))?
                .to_string();
            let credential_value = value
                .get("credential")
                .ok_or_else(|| anyhow::anyhow!("malformed list entry: missing `credential`"))?
                .clone();
            let credential = serde_json::from_value(credential_value)
                .map_err(|e| anyhow::anyhow!("deserialize credential: {e}"))?;
            out.push((provider, credential));
        }
        Ok(out)
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
        let stored = json!({ "provider": "anthropic", "credential": cred });
        let mock = Arc::new(MockTrigger::new(vec![Ok(stored)]));
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

    #[tokio::test]
    async fn set_writes_state_set_with_correct_payload() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![Ok(Value::Null)]));
        let store = IiiStateCredentialStore::new(mock.clone());
        let cred = Credential::ApiKey { key: "sk-test".into() };

        store.set("anthropic", cred.clone()).await?;

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "state::set");
        assert_eq!(calls[0].payload["scope"], SCOPE);
        assert_eq!(calls[0].payload["key"], "credential:anthropic");
        assert_eq!(calls[0].payload["value"]["provider"], "anthropic");
        assert_eq!(calls[0].payload["value"]["credential"], serde_json::to_value(&cred)?);
        Ok(())
    }

    #[tokio::test]
    async fn set_returns_err_on_trigger_failure() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler("boom".into()))]));
        let store = IiiStateCredentialStore::new(mock);
        let cred = Credential::ApiKey { key: "k".into() };
        assert!(store.set("anthropic", cred).await.is_err());
    }

    #[tokio::test]
    async fn clear_calls_state_delete_with_correct_payload() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![Ok(Value::Null)]));
        let store = IiiStateCredentialStore::new(mock.clone());

        store.clear("anthropic").await?;

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "state::delete");
        assert_eq!(calls[0].payload["scope"], SCOPE);
        assert_eq!(calls[0].payload["key"], "credential:anthropic");
        Ok(())
    }

    #[tokio::test]
    async fn clear_returns_err_on_trigger_failure() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler("boom".into()))]));
        let store = IiiStateCredentialStore::new(mock);
        assert!(store.clear("anthropic").await.is_err());
    }

    #[tokio::test]
    async fn list_returns_all_credentials_in_scope() -> anyhow::Result<()> {
        let cred_a = Credential::ApiKey { key: "ka".into() };
        let cred_b = Credential::ApiKey { key: "kb".into() };
        let response = json!([
            { "provider": "anthropic", "credential": cred_a.clone() },
            { "provider": "openai", "credential": cred_b.clone() },
        ]);
        let mock = Arc::new(MockTrigger::new(vec![Ok(response)]));
        let store = IiiStateCredentialStore::new(mock.clone());

        let mut listed = store.list().await?;
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            listed,
            vec![("anthropic".to_string(), cred_a), ("openai".to_string(), cred_b)],
        );

        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls[0].function_id, "state::list");
        assert_eq!(calls[0].payload["scope"], SCOPE);
        Ok(())
    }

    #[tokio::test]
    async fn list_returns_empty_on_empty_scope() -> anyhow::Result<()> {
        let mock = Arc::new(MockTrigger::new(vec![Ok(json!([]))]));
        let store = IiiStateCredentialStore::new(mock);
        assert!(store.list().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn list_returns_err_on_trigger_failure() {
        let mock = Arc::new(MockTrigger::new(vec![Err(IIIError::Handler("boom".into()))]));
        let store = IiiStateCredentialStore::new(mock);
        assert!(store.list().await.is_err());
    }
}
