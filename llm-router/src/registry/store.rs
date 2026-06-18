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
use crate::types::errors::{is_function_not_found, RouterCode, RouterError};
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

/// Outcome of `upsert`: the stored record, the raw registration token (it
/// exists nowhere else), and whether this registration recovered a previously
/// down provider — the caller emits `op:"available"` when it did.
pub struct Upserted {
    pub record: ProviderRecord,
    pub token: String,
    pub availability_recovered: bool,
}

/// Pure record assembly for upsert (no I/O — persistence is the caller's job).
/// Token hash and registered_at are preserved across a re-register; the rest
/// is recomposed from the new declaration.
fn build_record(
    existing: Option<&ProviderRecord>,
    declaration: ProviderDeclaration,
    worker_id: Option<String>,
    raw_token: &str,
) -> ProviderRecord {
    ProviderRecord {
        token_hash: existing
            .map(|e| e.token_hash.clone())
            .unwrap_or_else(|| hash_token(raw_token)),
        worker_id: worker_id.or_else(|| existing.and_then(|e| e.worker_id.clone())),
        // A (re)registering provider is connected and serving. Registration is
        // the source of truth for "up"; a dispatch-time function_not_found
        // flips this back down (chat.rs).
        available: true,
        registered_at: existing.map(|e| e.registered_at).unwrap_or_else(now_ms),
        declaration,
    }
}

/// Whether this registration brings a previously-down provider back up. A fresh
/// register is not a recovery (the `op:"register"` event already signals
/// presence); only a known provider whose `available` flag was false
/// transitioning to true is — and that transition needs its own `op:"available"`
/// event so availability subscribers don't stay stuck on the prior "unavailable".
fn availability_recovered(existing: Option<&ProviderRecord>) -> bool {
    matches!(existing.map(|e| e.available), Some(false))
}

impl RegistryStore {
    pub fn new(iii: III) -> Self {
        Self {
            iii,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), IIIError> {
        // No iii-state worker on this engine (the registry-publish flow boots
        // against a bare `workers: []` engine to collect the interface):
        // start empty. Safe to tolerate exactly this error class — with no
        // state worker, persists can't overwrite the stored snapshot either.
        let stored = match state_get(&self.iii, REGISTRY_KEY).await {
            Err(e) if is_function_not_found(&e) => {
                eprintln!("[llm-router] no iii-state worker; registry starts empty");
                return Ok(());
            }
            other => other?,
        };
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
    /// First register binds (mints a token, persists its hash); later
    /// registers must present the raw token. Returns the raw token — it
    /// exists nowhere else.
    pub async fn upsert(
        &self,
        declaration: ProviderDeclaration,
        worker_id: Option<String>,
        token: Option<String>,
    ) -> Result<Upserted, RouterError> {
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
        let recovered = availability_recovered(existing);
        let record = build_record(existing, declaration, worker_id, &raw_token);
        records.insert(record.declaration.id.clone(), record.clone());
        self.persist(&records).await.map_err(|e| {
            RouterError::new(
                RouterCode::InvalidRequest,
                format!("registry persist failed: {e}"),
            )
        })?;
        Ok(Upserted {
            record,
            token: raw_token,
            availability_recovered: recovered,
        })
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
    use serde_json::json;

    fn decl(id: &str) -> ProviderDeclaration {
        serde_json::from_value(json!({ "id": id })).expect("minimal declaration")
    }

    // F9: the engine keys topology events by a per-connection UUID while the
    // registry stores the provider's self-declared name, so the availability
    // handler can never resolve an event to a provider. A provider that just
    // (re)registered is, by definition, connected and serving — registration
    // is the source of truth for "up"; dispatch-time function_not_found is the
    // source of truth for "down".
    #[test]
    fn fresh_registration_is_available() {
        let record = build_record(None, decl("anthropic"), Some("w-1".into()), "tok");
        assert!(
            record.available,
            "a provider that just registered must be marked available"
        );
    }

    #[test]
    fn reregistration_restores_a_downed_provider() {
        let down = build_record(None, decl("anthropic"), Some("w-1".into()), "tok");
        let down = ProviderRecord {
            available: false, // a prior dispatch failure flipped it down
            ..down
        };
        let back = build_record(Some(&down), decl("anthropic"), Some("w-1".into()), "tok");
        assert!(
            back.available,
            "re-registering brings a downed provider back up"
        );
    }

    // A down→up transition on re-register must be reported so the register
    // handler can emit op:"available"; a fresh or already-up register must not.
    #[test]
    fn down_to_up_reregistration_is_a_recovery() {
        let down = ProviderRecord {
            available: false,
            ..build_record(None, decl("anthropic"), Some("w-1".into()), "tok")
        };
        assert!(availability_recovered(Some(&down)));
    }

    #[test]
    fn fresh_registration_is_not_a_recovery() {
        assert!(!availability_recovered(None));
    }

    #[test]
    fn reregistering_an_up_provider_is_not_a_recovery() {
        let up = build_record(None, decl("anthropic"), Some("w-1".into()), "tok"); // available: true
        assert!(!availability_recovered(Some(&up)));
    }
}
