//! Durable binding storage, in the state worker under `harness_binding`.
//!
//! No in-memory cache, deliberately. Workers registering the same id
//! load-balance, so two harness instances can serve the same mesh; a cached
//! "still live" binding in instance B after instance A retired it would
//! double-fire a `once`. The store IS the authority, and every fire pays one
//! private state read for it — the same round trip the fire already makes to
//! read the owner's turn record.

use iii_sdk::IIIClient;
use serde_json::Value;

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

#[derive(Debug)]
pub enum AttachOutcome {
    Attached(Box<Binding>),
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveOutcome {
    Reserved,
    Capacity,
    Exhausted,
}

#[derive(Clone)]
pub struct BindingStore {
    iii: std::sync::Arc<IIIClient>,
    timeout_ms: u64,
    events: crate::events::TurnEvents,
}

impl BindingStore {
    pub fn new(
        iii: std::sync::Arc<IIIClient>,
        timeout_ms: u64,
        events: crate::events::TurnEvents,
    ) -> Self {
        Self {
            iii,
            timeout_ms,
            events,
        }
    }

    pub async fn get(&self, id: &str) -> Result<Option<Binding>, HarnessError> {
        state::get_binding(&self.iii, id, self.timeout_ms).await
    }

    pub async fn delete(&self, id: &str) -> Result<(), HarnessError> {
        const ATTEMPTS: usize = 8;
        for _ in 0..ATTEMPTS {
            let Some(binding) = self.get(id).await? else {
                return Ok(());
            };
            if self.delete_if_unchanged(&binding).await? {
                return Ok(());
            }
        }
        Err(HarnessError::State(format!(
            "binding {id} moved during {ATTEMPTS} delete attempts"
        )))
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
        let now = crate::types::message::AgentMessage::now_ms();
        Ok(self
            .list_for_owner(session_id)
            .await?
            .into_iter()
            .find(|b| is_live_acknowledged_duplicate(b, dedup_key, now))
            .map(|b| b.id))
    }

    /// Insert a placeholder and atomically reserve one of the owner's 64
    /// slots. The owner index stores ids, not a counter, so retries and
    /// rollbacks cannot silently drift it.
    pub async fn reserve(&self, binding: &Binding) -> Result<ReserveOutcome, HarnessError> {
        let encoded = serde_json::to_value(binding)
            .map_err(|e| HarnessError::State(format!("binding serialize: {e}")))?;
        match state::cas_binding(&self.iii, None, binding, self.timeout_ms).await? {
            None => {}
            Some(current) if current == encoded => {}
            Some(_) => {
                return Err(HarnessError::State(format!(
                    "binding id {} already exists",
                    binding.id
                )))
            }
        }

        match self.reserve_owner_slot(binding).await {
            Ok(ReserveOutcome::Reserved) => {
                self.events
                    .emit_triggers_changed(&binding.owner.session_id)
                    .await;
                Ok(ReserveOutcome::Reserved)
            }
            Ok(outcome) => {
                let _ = self.delete_if_unchanged(binding).await;
                Ok(outcome)
            }
            Err(e) => {
                let _ = self.delete_if_unchanged(binding).await;
                Err(e)
            }
        }
    }

    /// Attach the engine id without overwriting an early fire's increment.
    pub async fn attach_trigger_id(
        &self,
        binding: &Binding,
        trigger_id: &str,
    ) -> Result<AttachOutcome, HarnessError> {
        const ATTEMPTS: usize = 8;
        let mut expected = binding.clone();
        for _ in 0..ATTEMPTS {
            if let Some(current) = expected.trigger_id.as_deref() {
                if current == trigger_id {
                    return Ok(AttachOutcome::Attached(Box::new(expected)));
                }
                return Err(HarnessError::State(format!(
                    "binding {} already belongs to engine trigger {current}",
                    binding.id
                )));
            }
            let mut next = expected.clone();
            next.trigger_id = Some(trigger_id.to_string());
            match state::cas_binding(&self.iii, Some(&expected), &next, self.timeout_ms).await? {
                None => {
                    self.events
                        .emit_triggers_changed(&binding.owner.session_id)
                        .await;
                    return Ok(AttachOutcome::Attached(Box::new(next)));
                }
                Some(current) if current.is_null() => return Ok(AttachOutcome::Gone),
                Some(current) => {
                    expected = serde_json::from_value(current).map_err(|e| {
                        HarnessError::State(format!(
                            "binding {} is unreadable while attaching trigger id: {e}",
                            binding.id
                        ))
                    })?;
                }
            }
        }
        Err(HarnessError::State(format!(
            "binding {} moved during {ATTEMPTS} trigger-id attach attempts",
            binding.id
        )))
    }

    /// Atomic compare-and-delete used by expiry and every ordinary teardown.
    /// `false` means a racing fire changed the record first.
    pub async fn delete_if_unchanged(&self, binding: &Binding) -> Result<bool, HarnessError> {
        let deleted = state::cas_delete_binding(&self.iii, binding, self.timeout_ms).await?;
        if deleted {
            self.events
                .emit_triggers_changed(&binding.owner.session_id)
                .await;
            if let Err(error) = self
                .release_owner_slot(&binding.owner.session_id, &binding.id)
                .await
            {
                // The record deletion is authoritative. A later reservation
                // rebuilds this id set from live records, so bookkeeping
                // cleanup must not turn a successful retirement into a retry
                // that suppresses its owner notification.
                tracing::warn!(
                    binding = %binding.id,
                    owner = %binding.owner.session_id,
                    error = %error,
                    "binding retired but owner capacity cleanup failed"
                );
            }
        }
        Ok(deleted)
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
                None => {
                    self.events
                        .emit_triggers_changed(&binding.owner.session_id)
                        .await;
                    return Ok(ClaimOutcome::Claimed(Box::new(next)));
                }
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

    async fn reserve_owner_slot(&self, binding: &Binding) -> Result<ReserveOutcome, HarnessError> {
        const ATTEMPTS: usize = 8;
        let owner = &binding.owner.session_id;
        for _ in 0..ATTEMPTS {
            let current = state::state_get(
                &self.iii,
                state::BINDING_OWNER_SCOPE,
                owner,
                self.timeout_ms,
            )
            .await?;
            let expected = (!current.is_null()).then(|| current.clone());
            let reserved = if current.is_null() {
                Vec::new()
            } else {
                parse_owner_ids(current.clone(), owner)?
            };
            let bindings = self.list_for_owner(owner).await?;
            let now = crate::types::message::AgentMessage::now_ms();
            let (ids, outcome) = select_owner_slot(reserved, bindings, binding, now);
            let next = serde_json::to_value(&ids)
                .map_err(|e| HarnessError::State(format!("owner index serialize: {e}")))?;
            if expected.as_ref() == Some(&next) {
                return Ok(outcome);
            }
            if state::cas_value(
                &self.iii,
                state::BINDING_OWNER_SCOPE,
                owner,
                expected,
                next,
                self.timeout_ms,
            )
            .await?
            .is_none()
            {
                return Ok(outcome);
            }
        }
        Err(HarnessError::State(format!(
            "binding owner {owner} lost {ATTEMPTS} capacity reservation attempts"
        )))
    }

    async fn release_owner_slot(&self, owner: &str, id: &str) -> Result<(), HarnessError> {
        const ATTEMPTS: usize = 8;
        for _ in 0..ATTEMPTS {
            let current = state::state_get(
                &self.iii,
                state::BINDING_OWNER_SCOPE,
                owner,
                self.timeout_ms,
            )
            .await?;
            if current.is_null() {
                return Ok(());
            }
            let mut ids = parse_owner_ids(current.clone(), owner)?;
            let old_len = ids.len();
            ids.retain(|item| item != id);
            if ids.len() == old_len {
                return Ok(());
            }
            let next = serde_json::to_value(ids)
                .map_err(|e| HarnessError::State(format!("owner index serialize: {e}")))?;
            if state::cas_value(
                &self.iii,
                state::BINDING_OWNER_SCOPE,
                owner,
                Some(current),
                next,
                self.timeout_ms,
            )
            .await?
            .is_none()
            {
                return Ok(());
            }
        }
        Err(HarnessError::State(format!(
            "binding owner {owner} lost {ATTEMPTS} capacity release attempts"
        )))
    }
}

fn is_live_acknowledged_duplicate(binding: &Binding, key: &Value, now: i64) -> bool {
    binding.trigger_id.is_some()
        && !binding.is_exhausted(now)
        && binding.dedup_key.as_ref() == Some(key)
}

fn owner_ids(reserved: Vec<String>, mut bindings: Vec<Binding>, now: i64) -> Vec<String> {
    bindings.retain(|binding| !binding.is_exhausted(now));
    let live: std::collections::HashSet<&str> =
        bindings.iter().map(|binding| binding.id.as_str()).collect();
    let mut ids: Vec<String> = reserved
        .into_iter()
        .filter(|id| live.contains(id.as_str()))
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_BINDINGS_PER_SESSION);

    // Never evict a live reservation another registration already won.
    // Fill remaining slots deterministically, preferring acknowledged legacy
    // records over concurrent placeholders.
    bindings.sort_by(|a, b| {
        (a.trigger_id.is_none(), a.created_at, a.id.as_str()).cmp(&(
            b.trigger_id.is_none(),
            b.created_at,
            b.id.as_str(),
        ))
    });
    for binding in bindings {
        if ids.len() == MAX_BINDINGS_PER_SESSION {
            break;
        }
        if !ids.contains(&binding.id) {
            ids.push(binding.id);
        }
    }
    ids.sort();
    ids
}

fn select_owner_slot(
    reserved: Vec<String>,
    bindings: Vec<Binding>,
    candidate: &Binding,
    now: i64,
) -> (Vec<String>, ReserveOutcome) {
    let ids = owner_ids(reserved, bindings, now);
    let outcome = if candidate.is_exhausted(now) {
        ReserveOutcome::Exhausted
    } else if ids.contains(&candidate.id) {
        ReserveOutcome::Reserved
    } else {
        ReserveOutcome::Capacity
    };
    (ids, outcome)
}

fn parse_owner_ids(value: Value, owner: &str) -> Result<Vec<String>, HarnessError> {
    serde_json::from_value(value)
        .map_err(|e| HarnessError::State(format!("binding owner index {owner} is unreadable: {e}")))
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

    #[test]
    fn duplicate_requires_an_acknowledged_live_binding() {
        let key = json!({ "trigger_type": "state" });
        let mut b = binding("a", "s1", Some(key.clone()));
        assert!(!is_live_acknowledged_duplicate(&b, &key, 0));

        b.trigger_id = Some("trig_1".into());
        assert!(is_live_acknowledged_duplicate(&b, &key, 0));

        b.lifecycle.once = true;
        b.fires = 1;
        assert!(!is_live_acknowledged_duplicate(&b, &key, 0));
    }

    #[test]
    fn owner_index_is_a_deduplicated_set() {
        let ids = owner_ids(
            vec![],
            vec![
                binding("b", "s1", None),
                binding("a", "s1", None),
                binding("a", "s1", None),
            ],
            0,
        );
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn owner_index_keeps_live_reservations_before_new_placeholders() {
        let mut all: Vec<_> = (0..MAX_BINDINGS_PER_SESSION)
            .map(|n| binding(&format!("old_{n:02}"), "s1", None))
            .collect();
        all.push(binding("new", "s1", None));
        let reserved = all[..MAX_BINDINGS_PER_SESSION]
            .iter()
            .map(|binding| binding.id.clone())
            .collect();
        let ids = owner_ids(reserved, all, 0);
        assert_eq!(ids.len(), MAX_BINDINGS_PER_SESSION);
        assert!(!ids.contains(&"new".to_string()));
    }

    #[test]
    fn reservation_selection_distinguishes_expiry_from_capacity() {
        let mut candidate = binding("new", "s1", None);
        candidate.lifecycle.expires_at = Some(100);

        let (ids, outcome) = select_owner_slot(vec![], vec![candidate.clone()], &candidate, 99);
        assert_eq!(outcome, ReserveOutcome::Reserved);
        assert_eq!(ids, vec!["new"]);

        let (ids, outcome) = select_owner_slot(vec![], vec![candidate.clone()], &candidate, 100);
        assert_eq!(outcome, ReserveOutcome::Exhausted);
        assert!(ids.is_empty());

        candidate.lifecycle.expires_at = None;
        let mut full: Vec<_> = (0..MAX_BINDINGS_PER_SESSION)
            .map(|n| binding(&format!("old_{n:02}"), "s1", None))
            .collect();
        let reserved = full.iter().map(|binding| binding.id.clone()).collect();
        full.push(candidate.clone());
        let (_, outcome) = select_owner_slot(reserved, full, &candidate, 100);
        assert_eq!(outcome, ReserveOutcome::Capacity);
    }
}
