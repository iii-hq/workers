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
use crate::types::errors::{sanitize_failure_message, RouterFailure};
use crate::types::router::{
    CatalogState, CredentialRequirement, CredentialState, ProviderDeclaration, ProviderDiagnostic,
    ProviderStatusRequest,
};
use iii_sdk::{errors::Error, IIIClient};
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
    #[serde(default)]
    pub diagnostic: ProviderDiagnostic,
}

pub struct RegistryStore {
    iii: IIIClient,
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
    let mut diagnostic = existing.map(|e| e.diagnostic.clone()).unwrap_or_default();
    diagnostic.credential_state = match declaration.credential_requirement {
        CredentialRequirement::External => CredentialState::External,
        CredentialRequirement::Optional => CredentialState::Ready,
        CredentialRequirement::Required => diagnostic.credential_state,
    };
    if declaration.supports_model_listing.unwrap_or(false)
        && diagnostic.catalog_state == CatalogState::Unknown
    {
        diagnostic.catalog_state = CatalogState::Discovering;
    }
    diagnostic.updated_at = now_ms();
    diagnostic.stale = false;
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
        diagnostic,
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
    pub fn new(iii: IIIClient) -> Self {
        Self {
            iii,
            records: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self) -> Result<(), Error> {
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
        // Persisted availability is stale across a router restart: a provider
        // that died while the router was down would be restored as "up" and
        // stay wrongly listed until a dispatch burned on it (F9: topology
        // events can't be resolved to providers). Restore every record DOWN
        // and let the sources of truth flip it up: providers re-declare on
        // `router::ready` (upsert emits the op:"available" recovery), and a
        // successful dispatch heals it too (chat.rs `Done` arm). Not persisted
        // here — flags re-persist on their next real change.
        for rec in records.values_mut() {
            rec.available = false;
            rec.diagnostic.stale = true;
        }
        Ok(())
    }

    async fn persist(&self, records: &HashMap<String, ProviderRecord>) -> Result<(), Error> {
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
        let _ = self.persist(&records).await; // best-effort persist of a flag flip
        true
    }

    /// Merge a provider-authored diagnostic update after verifying ownership.
    pub async fn update_diagnostic(
        &self,
        req: ProviderStatusRequest,
    ) -> Result<ProviderDiagnostic, RouterError> {
        self.verify_token(&req.id, req.token.as_deref()).await?;
        let mut records = self.records.lock().await;
        let rec = records.get_mut(&req.id).expect("verified record exists");
        if let Some(state) = req.credential_state {
            rec.diagnostic.credential_state = state;
        }
        if let Some(state) = req.catalog_state {
            rec.diagnostic.catalog_state = state;
        }
        if req.clear_failure {
            rec.diagnostic.last_failure = None;
        } else if let Some(mut failure) = req.failure {
            failure.message = sanitize_failure_message(&failure.message);
            if !RouterCode::is_known(&failure.code) {
                failure.code = RouterCode::UpstreamUnavailable.as_str().to_string();
            }
            failure.provider = Some(req.id.clone());
            rec.diagnostic.last_failure = Some(failure);
        }
        rec.diagnostic.updated_at = now_ms();
        rec.diagnostic.stale = false;
        let diagnostic = rec.diagnostic.clone();
        self.persist(&records).await.map_err(|e| {
            RouterError::new(
                RouterCode::InvalidRequest,
                format!("registry persist failed: {e}"),
            )
        })?;
        Ok(diagnostic)
    }

    /// Router-authored runtime evidence (chat/discovery); no provider token is
    /// involved because this code runs inside the registry owner.
    pub async fn set_runtime_diagnostic(
        &self,
        id: &str,
        failure: Option<RouterFailure>,
        catalog_state: Option<CatalogState>,
    ) {
        let mut records = self.records.lock().await;
        let Some(rec) = records.get_mut(id) else {
            return;
        };
        rec.diagnostic.last_failure = failure.map(|mut failure| {
            failure.message = sanitize_failure_message(&failure.message);
            failure.provider = Some(id.to_string());
            failure
        });
        if let Some(state) = catalog_state {
            rec.diagnostic.catalog_state = state;
        }
        rec.diagnostic.updated_at = now_ms();
        rec.diagnostic.stale = false;
        if let Err(error) = self.persist(&records).await {
            tracing::error!(provider = %id, %error, "failed to persist provider diagnostic");
        }
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

    #[test]
    fn registration_initializes_operator_diagnostics_without_stale_evidence() {
        let mut declaration = decl("openai-codex");
        declaration.credential_requirement = CredentialRequirement::External;
        declaration.supports_model_listing = Some(true);
        let record = build_record(None, declaration, Some("w-1".into()), "tok");
        assert_eq!(
            record.diagnostic.credential_state,
            CredentialState::External
        );
        assert_eq!(record.diagnostic.catalog_state, CatalogState::Discovering);
        assert!(!record.diagnostic.stale);
        assert!(record.diagnostic.updated_at > 0);
    }
}
