//! Durable provider registry. Single writer: the records Mutex is held across
//! mutate + state::set (spec § Registration lifecycle, "Serialized merges").
//! Persistence is iii state (`state::get`/`state::set` engine functions) under
//! scope "llm-router"; the router is the only writer of its state keys
//! (single-instance worker).
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::ProviderDeclaration;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::bus::{Bus, BusError};

pub const STATE_SCOPE: &str = "llm-router";
const REGISTRY_KEY: &str = "registry";

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub declaration: ProviderDeclaration,
    pub token_hash: String, // sha256 of the registration token; raw token never persisted
    pub worker_id: Option<String>,
    pub available: bool,
    pub registered_at: i64,
}

pub struct RegistryStore {
    bus: Arc<dyn Bus>,
    records: Mutex<HashMap<String, ProviderRecord>>,
}

impl RegistryStore {
    pub fn new(bus: Arc<dyn Bus>) -> Self {
        Self {
            bus,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), BusError> {
        let stored = self
            .bus
            .trigger(
                "state::get",
                json!({ "scope": STATE_SCOPE, "key": REGISTRY_KEY }),
                None,
            )
            .await?;
        let mut records = self.records.lock().await;
        *records = serde_json::from_value(stored).unwrap_or_default();
        Ok(())
    }

    async fn persist(&self, records: &HashMap<String, ProviderRecord>) -> Result<(), BusError> {
        let value = serde_json::to_value(records).unwrap_or_default();
        self.bus
            .trigger(
                "state::set",
                json!({ "scope": STATE_SCOPE, "key": REGISTRY_KEY, "value": value }),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<ProviderRecord> {
        self.records.lock().await.get(id).cloned()
    }
    pub async fn list(&self) -> Vec<ProviderRecord> {
        self.records.lock().await.values().cloned().collect()
    }
    pub async fn ids(&self) -> Vec<String> {
        self.records.lock().await.keys().cloned().collect()
    }
    pub async fn providers_for_worker(&self, worker_id: &str) -> Vec<String> {
        self.records
            .lock()
            .await
            .values()
            .filter(|r| r.worker_id.as_deref() == Some(worker_id))
            .map(|r| r.declaration.id.clone())
            .collect()
    }

    /// First register binds (mints a token, persists its hash); later
    /// registers must present the raw token. Returns the raw token — it
    /// exists nowhere else.
    pub async fn upsert(
        &self,
        declaration: ProviderDeclaration,
        worker_id: Option<String>,
        token: Option<String>,
    ) -> Result<(ProviderRecord, String), RouterError> {
        let mut records = self.records.lock().await; // serialized writer
        let existing = records.get(&declaration.id);
        if let Some(existing) = existing {
            let presented = token.as_deref().map(hash_token);
            if presented.as_deref() != Some(existing.token_hash.as_str()) {
                return Err(RouterError::new(
                    RouterCode::RegistrationRejected,
                    format!(
                        "provider {} is bound to another worker; re-binding is an operator action",
                        declaration.id
                    ),
                ));
            }
        }
        let raw_token = token.unwrap_or_else(|| Uuid::new_v4().to_string());
        let record = ProviderRecord {
            token_hash: existing
                .map(|e| e.token_hash.clone())
                .unwrap_or_else(|| hash_token(&raw_token)),
            worker_id: worker_id.or_else(|| existing.and_then(|e| e.worker_id.clone())),
            available: existing.map(|e| e.available).unwrap_or(false),
            registered_at: existing.map(|e| e.registered_at).unwrap_or_else(now_ms),
            declaration,
        };
        records.insert(record.declaration.id.clone(), record.clone());
        self.persist(&records).await.map_err(|e| {
            RouterError::new(
                RouterCode::InvalidRequest,
                format!("registry persist failed: {e}"),
            )
        })?;
        Ok((record, raw_token))
    }

    /// Token gate for resolve / reconcile / update_credential (and re-register).
    pub async fn verify_token(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<ProviderRecord, RouterError> {
        let records = self.records.lock().await;
        let rec = records.get(id).ok_or_else(|| {
            RouterError::new(
                RouterCode::UnknownProvider,
                format!("unknown provider {id}"),
            )
        })?;
        match token {
            Some(t) if hash_token(t) == rec.token_hash => Ok(rec.clone()),
            _ => Err(RouterError::new(
                RouterCode::RegistrationRejected,
                format!("provider {id}: registration token mismatch"),
            )),
        }
    }

    /// Returns true when the flag actually changed (callers emit on change only).
    pub async fn set_availability(&self, id: &str, available: bool) -> bool {
        let mut records = self.records.lock().await;
        let Some(rec) = records.get_mut(id) else {
            return false;
        };
        if rec.available == available {
            return false;
        }
        rec.available = available;
        let snapshot = records.clone();
        drop(records);
        let _ = self.persist(&snapshot).await; // best-effort persist of a flag flip
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::fake_bus::FakeBus;
    use crate::types::errors::RouterCode;
    use crate::types::router::ProviderDeclaration;

    fn decl(id: &str) -> ProviderDeclaration {
        ProviderDeclaration {
            id: id.into(),
            display_name: Some(id.into()),
            credential_env_var: None,
            defaults: None,
            config_schema: None,
            supports_model_listing: Some(false),
            models: None,
            worker_id: None,
        }
    }

    #[tokio::test]
    async fn mints_on_first_register_persists_hash_only_and_restores() {
        let bus = FakeBus::new();
        let store = RegistryStore::new(bus.clone());
        store.load().await.unwrap();
        let (record, token) = store
            .upsert(decl("anthropic"), Some("w1".into()), None)
            .await
            .unwrap();
        assert!(token.len() >= 16);
        assert_ne!(record.token_hash, token);

        // restart: a fresh store restores from state; raw token absent, still verifies
        let store2 = RegistryStore::new(bus.clone());
        store2.load().await.unwrap();
        let restored = store2.get("anthropic").await.unwrap();
        assert_eq!(restored.token_hash, record.token_hash);
        assert!(!serde_json::to_string(&restored).unwrap().contains(&token));
        assert!(store2.verify_token("anthropic", Some(&token)).await.is_ok());
    }

    #[tokio::test]
    async fn reregister_requires_the_token_and_merges_the_declaration() {
        let bus = FakeBus::new();
        let store = RegistryStore::new(bus);
        store.load().await.unwrap();
        let (first, token) = store
            .upsert(decl("a"), Some("w1".into()), None)
            .await
            .unwrap();

        let mut renamed = decl("a");
        renamed.display_name = Some("Anthropic".into());
        let (second, token2) = store
            .upsert(renamed, Some("w1".into()), Some(token.clone()))
            .await
            .unwrap();
        assert_eq!(token2, token);
        assert_eq!(second.token_hash, first.token_hash);
        assert_eq!(
            store
                .get("a")
                .await
                .unwrap()
                .declaration
                .display_name
                .as_deref(),
            Some("Anthropic")
        );

        // takeover without / with wrong token is rejected
        let err = store
            .upsert(decl("a"), Some("evil".into()), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, RouterCode::RegistrationRejected);
        let err = store
            .upsert(decl("a"), Some("evil".into()), Some("wrong".into()))
            .await
            .unwrap_err();
        assert_eq!(err.code, RouterCode::RegistrationRejected);
    }

    #[tokio::test]
    async fn verify_token_gates_and_availability_flips_persist() {
        let bus = FakeBus::new();
        let store = RegistryStore::new(bus);
        store.load().await.unwrap();
        let (_, token) = store
            .upsert(decl("a"), Some("w1".into()), None)
            .await
            .unwrap();
        assert_eq!(
            store
                .verify_token("missing", Some("x"))
                .await
                .unwrap_err()
                .code,
            RouterCode::UnknownProvider
        );
        assert_eq!(
            store
                .verify_token("a", Some("nope"))
                .await
                .unwrap_err()
                .code,
            RouterCode::RegistrationRejected
        );
        assert_eq!(
            store.verify_token("a", None).await.unwrap_err().code,
            RouterCode::RegistrationRejected
        );
        assert!(store.verify_token("a", Some(&token)).await.is_ok());

        assert!(store.set_availability("a", true).await); // changed
        assert!(!store.set_availability("a", true).await); // no-op
        assert_eq!(store.providers_for_worker("w1").await, vec!["a"]);
    }

    #[tokio::test]
    async fn concurrent_registrations_of_different_ids_both_survive() {
        let bus = FakeBus::new();
        let store = std::sync::Arc::new(RegistryStore::new(bus));
        store.load().await.unwrap();
        let (s1, s2) = (store.clone(), store.clone());
        let (a, b) = tokio::join!(
            tokio::spawn(async move { s1.upsert(decl("a"), None, None).await }),
            tokio::spawn(async move { s2.upsert(decl("b"), None, None).await }),
        );
        a.unwrap().unwrap();
        b.unwrap().unwrap();
        let mut ids = store.ids().await;
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
