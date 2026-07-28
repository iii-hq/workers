//! Durable binding storage, in the state worker under `harness_binding`.
//!
//! No in-memory cache, deliberately. Workers registering the same id
//! load-balance, so two harness instances can serve the same mesh; a cached
//! "still live" binding in instance B after instance A retired it would
//! double-fire a `once`. The store IS the authority, and every fire pays one
//! `state::get` for it — the same round trip the fire already makes to read
//! the owner's turn record.

use iii_sdk::IIIClient;

use super::Binding;
use crate::error::HarnessError;
use crate::state;

/// Per-session ceiling on live bindings, unchanged from the registry it
/// replaces: a runaway registration loop is a real failure mode and the cap is
/// what turns it into an error the model can read.
pub const MAX_BINDINGS_PER_SESSION: usize = 64;

/// What happened when a fire tried to take its slot.
#[derive(Debug)]
pub enum ClaimOutcome {
    /// The slot is this delivery's; the record carries its ordinal. Boxed
    /// because it dwarfs the other two variants and this is returned on every
    /// fire.
    Claimed(Box<Binding>),
    /// The lifecycle was spent by whoever won the race.
    Exhausted,
    /// The binding was retired while this fire was in flight.
    Gone,
}

#[derive(Clone)]
pub struct BindingStore {
    iii: std::sync::Arc<IIIClient>,
    timeout_ms: u64,
}

impl BindingStore {
    pub fn new(iii: std::sync::Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    pub async fn get(&self, id: &str) -> Result<Option<Binding>, HarnessError> {
        state::get_binding(&self.iii, id, self.timeout_ms).await
    }

    pub async fn put(&self, binding: &Binding) -> Result<(), HarnessError> {
        state::put_binding(&self.iii, binding, self.timeout_ms).await
    }

    pub async fn delete(&self, id: &str) -> Result<(), HarnessError> {
        state::delete_binding(&self.iii, id, self.timeout_ms).await
    }

    pub async fn list(&self) -> Result<Vec<Binding>, HarnessError> {
        state::list_bindings(&self.iii, self.timeout_ms).await
    }

    pub async fn list_for_owner(&self, session_id: &str) -> Result<Vec<Binding>, HarnessError> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|b| b.owner.session_id == session_id)
            .collect())
    }

    /// The standing binding an identical re-registration should return instead
    /// of wiring a twin that double-delivers forever. Same rule as the
    /// registry's: same owner session, same canonicalised request.
    pub async fn find_duplicate(
        &self,
        session_id: &str,
        dedup_key: &serde_json::Value,
    ) -> Result<Option<String>, HarnessError> {
        Ok(self
            .list_for_owner(session_id)
            .await?
            .into_iter()
            .find(|b| b.dedup_key.as_ref() == Some(dedup_key))
            .map(|b| b.id))
    }

    /// Take this fire's slot, atomically. Returns the claimed record — the one
    /// whose `fires` is now this delivery's ordinal.
    ///
    /// Retries a bounded number of times: a losing racer re-reads what is
    /// actually stored and recomputes against it, so two simultaneous fires
    /// take slot N and N+1 rather than both taking N. Before this the claim was
    /// a `get` then a `put`, and a committed two-statement transaction
    /// dispatching both changes at once produced four claims and three
    /// deliveries — the fourth collided on the third's ordinal.
    pub async fn claim_fire(&self, binding: &Binding) -> Result<ClaimOutcome, HarnessError> {
        const ATTEMPTS: usize = 8;
        let mut expected = binding.clone();
        for _ in 0..ATTEMPTS {
            let now = crate::types::message::AgentMessage::now_ms();
            if expected.is_exhausted(now) {
                return Ok(ClaimOutcome::Exhausted);
            }
            let mut next = expected.clone();
            next.fires = expected.fires + 1;

            match state::cas_binding(&self.iii, Some(&expected), &next, self.timeout_ms).await? {
                None => return Ok(ClaimOutcome::Claimed(Box::new(next))),
                Some(current) => {
                    // Someone moved it. A record that is gone means the binding
                    // retired between the read and here — not an error, just
                    // nothing left to claim.
                    if current.is_null() {
                        return Ok(ClaimOutcome::Gone);
                    }
                    match serde_json::from_value::<Binding>(current) {
                        Ok(fresh) => expected = fresh,
                        Err(e) => {
                            return Err(HarnessError::State(format!(
                                "binding {} is unreadable mid-claim: {e}",
                                binding.id
                            )))
                        }
                    }
                }
            }
        }
        // Contention this sustained is not a race, it is a runaway: refuse the
        // fire rather than spin.
        Err(HarnessError::State(format!(
            "binding {} lost {ATTEMPTS} claim attempts in a row",
            binding.id
        )))
    }

    /// Whether the owner has room for another binding.
    pub async fn has_capacity(&self, session_id: &str) -> Result<bool, HarnessError> {
        Ok(self.list_for_owner(session_id).await?.len() < MAX_BINDINGS_PER_SESSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};
    use serde_json::json;

    fn binding(id: &str, owner: &str, dedup: Option<serde_json::Value>) -> Binding {
        Binding {
            id: id.into(),
            trigger_id: None,
            owner: OwnerScope {
                session_id: owner.into(),
                root_session_id: None,
            },
            target: BindingTarget::new("harness::send"),
            conditions: vec![],
            lifecycle: Lifecycle::default(),
            capability: None,
            causation: Causation::default(),
            dedup_key: dedup,
            fires: 0,
            created_at: 0,
        }
    }

    // The store's own filters are pure over a listing; exercise them directly
    // so the behaviour is pinned without a live state worker.
    fn owned<'a>(all: &'a [Binding], session: &str) -> Vec<&'a Binding> {
        all.iter()
            .filter(|b| b.owner.session_id == session)
            .collect()
    }

    #[test]
    fn owner_filter_separates_sessions() {
        let all = vec![
            binding("a", "s1", None),
            binding("b", "s2", None),
            binding("c", "s1", None),
        ];
        assert_eq!(owned(&all, "s1").len(), 2);
        assert_eq!(owned(&all, "s2").len(), 1);
        assert!(owned(&all, "s3").is_empty());
    }

    #[test]
    fn duplicate_matches_only_within_the_same_owner() {
        let key = json!({ "trigger_type": "state", "config": { "scope": "run" } });
        let all = vec![
            binding("a", "s1", Some(key.clone())),
            binding("b", "s2", Some(key.clone())),
        ];
        let found: Vec<_> = owned(&all, "s2")
            .into_iter()
            .filter(|b| b.dedup_key.as_ref() == Some(&key))
            .map(|b| b.id.clone())
            .collect();
        assert_eq!(found, vec!["b".to_string()]);
    }

    #[test]
    fn a_keyless_binding_never_matches_a_duplicate_probe() {
        let key = json!({ "trigger_type": "cron" });
        let all = vec![binding("a", "s1", None)];
        assert!(owned(&all, "s1")
            .into_iter()
            .all(|b| b.dedup_key.as_ref() != Some(&key)));
    }
}
