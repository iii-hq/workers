//! Compaction mutual exclusion (context-manager.md § State): a
//! `{nonce, ts}` claim under scope `context_lease`, keyed by
//! `options.lease_key` (e.g. a session id) or a hash of the message
//! set. TTL is enforced by readers — a claim older than the TTL reads
//! as free, folding crash recovery into acquisition. Ported from
//! harness `runtime/lease.ts`.

use sha2::{Digest, Sha256};

use crate::ports::{Clock, LeaseRecord, LeaseStore};
use crate::types::AgentMessage;

/// The single lease scope this worker writes (a subdirectory under the
/// configured `lease_dir`).
pub const LEASE_SCOPE: &str = "context_lease";

/// Default lease key: sha256 over the serialized message set, so two
/// callers compacting the same logical history contend even without an
/// explicit `lease_key`, while different histories never block each
/// other. sha2 (not a std hasher) keeps the key stable across
/// processes and Rust versions.
pub fn default_lease_key(messages: &[AgentMessage]) -> String {
    let mut hasher = Sha256::new();
    for message in messages {
        hasher.update(serde_json::to_string(message).unwrap_or_default());
        hasher.update([0u8]); // message separator
    }
    format!("{:x}", hasher.finalize())
}

fn is_active(record: Option<&LeaseRecord>, now_ms: i64, ttl_ms: i64) -> bool {
    record.is_some_and(|r| now_ms - r.ts < ttl_ms)
}

/// Try to acquire the lease. Returns the nonce to release with, or
/// `None` when another holder owns a live claim. Store failures read
/// as "not acquired" (never a win) — an outage must not let every
/// contender through, and a transient `busy` is retryable.
pub async fn acquire(
    store: &dyn LeaseStore,
    clock: &dyn Clock,
    key: &str,
    ttl_ms: i64,
) -> Option<String> {
    let now = clock.now_ms();
    match store.get(LEASE_SCOPE, key).await {
        Ok(existing) if is_active(existing.as_ref(), now, ttl_ms) => return None,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, key, "lease get failed; treating as busy");
            return None;
        }
    }

    let nonce = format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple());
    let claim = LeaseRecord {
        nonce: nonce.clone(),
        ts: now,
    };
    let prior = match store.swap(LEASE_SCOPE, key, claim).await {
        Ok(prior) => prior,
        Err(e) => {
            tracing::warn!(error = %e, key, "lease swap failed; never treating as a win");
            return None;
        }
    };

    // We clobbered a still-valid claim (always someone else's — our
    // nonce is fresh): restore it and bow out.
    if is_active(prior.as_ref(), now, ttl_ms) {
        if let Err(e) = store.set(LEASE_SCOPE, key, prior).await {
            tracing::warn!(error = %e, key, "lease restore failed");
        }
        return None;
    }
    Some(nonce)
}

/// Release only our own claim: a holder that lost its lease to TTL
/// takeover must not clear the new holder's claim.
pub async fn release(store: &dyn LeaseStore, key: &str, nonce: &str) {
    match store.get(LEASE_SCOPE, key).await {
        Ok(Some(stored)) if stored.nonce == nonce => {
            if let Err(e) = store.set(LEASE_SCOPE, key, None).await {
                tracing::warn!(error = %e, key, "lease clear failed");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, key, "lease release read failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_key_is_stable_and_input_sensitive() {
        let a: AgentMessage = serde_json::from_value(json!({
            "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1
        }))
        .unwrap();
        let b: AgentMessage = serde_json::from_value(json!({
            "role": "user", "content": [{ "type": "text", "text": "bye" }], "timestamp": 1
        }))
        .unwrap();

        let key_a1 = default_lease_key(std::slice::from_ref(&a));
        let key_a2 = default_lease_key(std::slice::from_ref(&a));
        let key_b = default_lease_key(std::slice::from_ref(&b));
        assert_eq!(key_a1, key_a2, "same messages must contend");
        assert_ne!(
            key_a1, key_b,
            "different messages must not block each other"
        );
        assert_eq!(key_a1.len(), 64, "sha256 hex");
    }

    #[test]
    fn concatenation_is_not_ambiguous() {
        // ["ab"] vs ["a", "b"]-style boundary games must not collide;
        // the separator byte keeps message boundaries in the hash.
        let one: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "ab" }], "timestamp": 1 }
        ]))
        .unwrap();
        let two: Vec<AgentMessage> = serde_json::from_value(json!([
            { "role": "user", "content": [{ "type": "text", "text": "a" }], "timestamp": 1 },
            { "role": "user", "content": [{ "type": "text", "text": "b" }], "timestamp": 1 }
        ]))
        .unwrap();
        assert_ne!(default_lease_key(&one), default_lease_key(&two));
    }

    #[test]
    fn activity_is_ttl_relative() {
        let rec = LeaseRecord {
            nonce: "n".into(),
            ts: 1_000,
        };
        assert!(is_active(Some(&rec), 1_500, 1_000)); // 500ms old, 1s TTL
        assert!(!is_active(Some(&rec), 2_500, 1_000)); // expired
        assert!(!is_active(None, 0, 1_000));
    }
}
