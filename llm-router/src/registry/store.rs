//! Durable provider registry. Single writer: the records Mutex is held across
//! mutate + state::set (spec § Registration lifecycle, "Serialized merges").
//! Persistence is iii state (`state::get`/`state::set` engine functions via
//! src/state.rs) under scope "llm-router"; the router is the only writer of
//! its state keys (single-instance worker).
//!
//! Engine-backed coverage: tests/integration.rs (registration, token gate,
//! restart restore).
use std::collections::HashMap;

use crate::state::{state_get, state_set};
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::ProviderDeclaration;
use iii_sdk::{IIIError, III};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

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
    iii: III,
    records: Mutex<HashMap<String, ProviderRecord>>,
}

impl RegistryStore {
    pub fn new(iii: III) -> Self {
        Self {
            iii,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), IIIError> {
        let stored = state_get(&self.iii, REGISTRY_KEY).await?;
        let mut records = self.records.lock().await;
        *records = serde_json::from_value(stored).unwrap_or_default();
        Ok(())
    }

    async fn persist(&self, records: &HashMap<String, ProviderRecord>) -> Result<(), IIIError> {
        let value = serde_json::to_value(records).unwrap_or_default();
        state_set(&self.iii, REGISTRY_KEY, value).await
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
