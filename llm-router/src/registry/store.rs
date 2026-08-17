//! Durable provider registry. Single writer: the records Mutex is held across
//! prepare + state::set + publish (spec § Registration lifecycle, "Serialized merges").
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
    /// Monotonic identity for one successful registration of this provider.
    /// Legacy snapshots deserialize as generation zero; their first
    /// re-registration advances to one.
    #[serde(default)]
    pub generation: u64,
}

pub struct RegistryStore {
    iii: IIIClient,
    records: Mutex<HashMap<String, ProviderRecord>>,
    #[cfg(test)]
    persist_result: Option<Result<(), Error>>,
}

/// Outcome of `upsert`: the stored record, the raw registration token (it
/// exists nowhere else), and whether this registration recovered a previously
/// down provider — the caller emits `op:"available"` when it did.
pub struct Upserted {
    pub record: ProviderRecord,
    pub token: String,
    pub availability_recovered: bool,
}

/// Token-gated registration assembled without changing durable or in-memory
/// state. The register handler may safely perform its other fallible setup
/// before passing this value to `commit_upsert`.
pub struct PreparedUpsert {
    record: ProviderRecord,
    token: String,
    expected: Option<RegistrationRevision>,
}

#[derive(Clone)]
struct RegistrationRevision {
    token_hash: String,
    generation: u64,
}

impl PreparedUpsert {
    pub fn record(&self) -> &ProviderRecord {
        &self.record
    }
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
        generation: existing
            .map(|e| e.generation.wrapping_add(1).max(1))
            .unwrap_or(1),
        declaration,
    }
}

fn revision(record: &ProviderRecord) -> RegistrationRevision {
    RegistrationRevision {
        token_hash: record.token_hash.clone(),
        generation: record.generation,
    }
}

fn revision_matches(
    current: Option<&ProviderRecord>,
    expected: Option<&RegistrationRevision>,
) -> bool {
    match (current, expected) {
        (None, None) => true,
        (Some(current), Some(expected)) => {
            current.token_hash == expected.token_hash && current.generation == expected.generation
        }
        _ => false,
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
            #[cfg(test)]
            persist_result: None,
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
        }
        Ok(())
    }

    async fn persist(&self, records: &HashMap<String, ProviderRecord>) -> Result<(), Error> {
        #[cfg(test)]
        if let Some(result) = &self.persist_result {
            return result.clone();
        }
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
    pub async fn prepare_upsert(
        &self,
        declaration: ProviderDeclaration,
        worker_id: Option<String>,
        token: Option<String>,
    ) -> Result<PreparedUpsert, RouterError> {
        let records = self.records.lock().await;
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
        let record = build_record(existing, declaration, worker_id, &raw_token);
        Ok(PreparedUpsert {
            record,
            token: raw_token,
            expected: existing.map(revision),
        })
    }

    /// Persist and publish a prepared registration if its predecessor is
    /// still current. The revision check makes the two-phase API safe even if
    /// a caller fails to hold the higher-level registration lock.
    pub async fn commit_upsert(&self, prepared: PreparedUpsert) -> Result<Upserted, RouterError> {
        let mut records = self.records.lock().await; // serialized writer
        let id = prepared.record.declaration.id.clone();
        let existing = records.get(&id);
        if !revision_matches(existing, prepared.expected.as_ref()) {
            return Err(RouterError::new(
                RouterCode::RegistrationRejected,
                format!("provider {id} changed while registration was being prepared; retry"),
            ));
        }
        let recovered = availability_recovered(existing);
        let mut next_records = records.clone();
        next_records.insert(id, prepared.record.clone());
        self.persist(&next_records).await.map_err(|e| {
            RouterError::new(
                RouterCode::InvalidRequest,
                format!("registry persist failed: {e}"),
            )
        })?;
        // Publish only after the durable write succeeds. In particular, a
        // failed first registration must not retain a hash for the raw token
        // that the caller never received.
        *records = next_records;
        Ok(Upserted {
            record: prepared.record,
            token: prepared.token,
            availability_recovered: recovered,
        })
    }

    pub async fn upsert(
        &self,
        declaration: ProviderDeclaration,
        worker_id: Option<String>,
        token: Option<String>,
    ) -> Result<Upserted, RouterError> {
        let prepared = self.prepare_upsert(declaration, worker_id, token).await?;
        self.commit_upsert(prepared).await
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

    /// Change availability only for the registration that originated an
    /// observed dispatch result. Availability is deliberately memory-only:
    /// persisted flags are stale after a restart and `load` always restores
    /// them as down. Avoiding a full-registry state write here also keeps a
    /// state-worker outage out of the dispatch hot path and prevents an old
    /// availability snapshot from overwriting a newer registration.
    /// Returns true when the in-memory flag changed.
    pub async fn set_availability_if_current(
        &self,
        id: &str,
        generation: u64,
        available: bool,
    ) -> bool {
        let mut records = self.records.lock().await;
        let Some(rec) = records.get(id) else {
            return false;
        };
        if rec.generation != generation || rec.available == available {
            return false;
        }
        records
            .get_mut(id)
            .expect("record came from the same snapshot")
            .available = available;
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

    fn store_with_persistence(result: Result<(), Error>) -> RegistryStore {
        RegistryStore {
            iii: IIIClient::new("ws://unused.invalid"),
            records: Mutex::new(HashMap::new()),
            persist_result: Some(result),
        }
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
        assert_eq!(record.generation, 1);
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
        assert_eq!(back.generation, down.generation + 1);
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
    fn legacy_record_without_generation_deserializes_as_zero() {
        let record: ProviderRecord = serde_json::from_value(json!({
            "declaration": { "id": "anthropic" },
            "token_hash": "hash",
            "worker_id": "w-legacy",
            "available": true,
            "registered_at": 1
        }))
        .expect("legacy registry snapshot remains readable");
        assert_eq!(record.generation, 0);
        let next = build_record(
            Some(&record),
            decl("anthropic"),
            Some("w-new".into()),
            "ignored",
        );
        assert_eq!(next.generation, 1);
    }

    #[test]
    fn reregistering_an_up_provider_is_not_a_recovery() {
        let up = build_record(None, decl("anthropic"), Some("w-1".into()), "tok"); // available: true
        assert!(!availability_recovered(Some(&up)));
    }

    #[tokio::test(start_paused = true)]
    async fn failed_persist_does_not_publish_registration_or_orphan_token() {
        // An unconnected client makes state::set time out. Tokio's paused
        // clock advances directly to the SDK timeout, so this stays a fast
        // unit test without a live engine.
        let store = RegistryStore::new(IIIClient::new("ws://unconnected.invalid"));

        for _ in 0..2 {
            let err = store
                .upsert(decl("anthropic"), Some("w-1".into()), None)
                .await
                .err()
                .expect("persistence must fail");
            assert_eq!(err.code, RouterCode::InvalidRequest);
            assert!(err.message.contains("registry persist failed"));
            assert!(
                store.get("anthropic").await.is_none(),
                "a failed write must not publish a record or its token hash"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn failed_persist_keeps_previous_registration_unchanged() {
        let store = RegistryStore::new(IIIClient::new("ws://unconnected.invalid"));
        let original = ProviderRecord {
            available: false,
            ..build_record(None, decl("anthropic"), Some("w-old".into()), "tok")
        };
        store
            .records
            .lock()
            .await
            .insert("anthropic".into(), original.clone());

        let mut replacement = decl("anthropic");
        replacement.display_name = Some("replacement".into());
        let err = store
            .upsert(replacement, Some("w-new".into()), Some("tok".into()))
            .await
            .err()
            .expect("persistence must fail");
        assert_eq!(err.code, RouterCode::InvalidRequest);

        let current = store
            .get("anthropic")
            .await
            .expect("the previous registration must remain");
        assert_eq!(current.token_hash, original.token_hash);
        assert_eq!(current.worker_id, original.worker_id);
        assert_eq!(current.registered_at, original.registered_at);
        assert_eq!(current.available, original.available);
        assert_eq!(current.declaration, original.declaration);
    }

    #[tokio::test]
    async fn stale_prepared_registration_is_rejected_after_another_commit() {
        let store = store_with_persistence(Ok(()));
        let stale = store
            .prepare_upsert(decl("anthropic"), Some("w-stale".into()), None)
            .await
            .expect("first prepare");
        let winner = store
            .prepare_upsert(decl("anthropic"), Some("w-winner".into()), None)
            .await
            .expect("concurrent prepare");
        let winner = store.commit_upsert(winner).await.expect("winner commits");

        let err = store
            .commit_upsert(stale)
            .await
            .err()
            .expect("stale candidate must lose the compare-and-set");
        assert_eq!(err.code, RouterCode::RegistrationRejected);
        let current = store.get("anthropic").await.expect("winner remains");
        assert_eq!(current.worker_id.as_deref(), Some("w-winner"));
        assert_eq!(current.generation, winner.record.generation);
    }

    #[tokio::test]
    async fn old_generation_cannot_change_reregistered_provider_availability() {
        let store = store_with_persistence(Ok(()));
        let first = store
            .upsert(decl("anthropic"), Some("w-1".into()), None)
            .await
            .expect("first registration");
        let old_generation = first.record.generation;
        let second = store
            .upsert(decl("anthropic"), Some("w-2".into()), Some(first.token))
            .await
            .expect("re-registration");
        assert!(second.record.generation > old_generation);

        assert!(
            !store
                .set_availability_if_current("anthropic", old_generation, false)
                .await,
            "a late function_not_found from the old registration must be ignored"
        );
        let current = store.get("anthropic").await.expect("provider remains");
        assert_eq!(current.generation, second.record.generation);
        assert!(current.available);
    }

    #[tokio::test]
    async fn availability_change_does_not_depend_on_state_persistence() {
        let store = store_with_persistence(Err(Error::Timeout));
        let record = build_record(None, decl("anthropic"), Some("w-1".into()), "tok");
        let generation = record.generation;
        store
            .records
            .lock()
            .await
            .insert("anthropic".into(), record);

        assert!(
            store
                .set_availability_if_current("anthropic", generation, false)
                .await
        );
        assert!(
            !store
                .get("anthropic")
                .await
                .expect("provider remains")
                .available,
            "availability must remain operational while durable state is unavailable"
        );
    }
}
