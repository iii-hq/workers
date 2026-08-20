//! Durable per-session A2UI state over the `state` worker.
//!
//! One state key equals one Harness session id. That gives the Console an
//! exact-key live subscription, so one browser tab never receives another
//! session's surface payload while still avoiding polling.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::protocol::SessionState;

pub const STATE_SCOPE: &str = "a2ui";
const STATE_TIMEOUT_MS: u64 = 10_000;

enum Backend {
    Bus(Arc<IIIClient>),
    Memory(std::sync::Mutex<HashMap<String, Value>>),
}

pub struct Store {
    backend: Backend,
    mutation_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}

impl Store {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            backend: Backend::Bus(iii),
            mutation_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            backend: Backend::Memory(std::sync::Mutex::new(HashMap::new())),
            mutation_locks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn mutation_guard(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.mutation_locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(session_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(session_id.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    async fn bus_call(
        iii: &Arc<IIIClient>,
        function_id: &str,
        payload: Value,
    ) -> Result<Value, String> {
        iii.trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await
        .map_err(|error| format!("{function_id} failed: {error}"))
    }

    pub async fn load(&self, session_id: &str) -> Result<SessionState, String> {
        let value = match &self.backend {
            Backend::Bus(iii) => {
                Self::bus_call(
                    iii,
                    "state::get",
                    json!({"scope": STATE_SCOPE, "key": session_id}),
                )
                .await?
            }
            Backend::Memory(values) => values
                .lock()
                .expect("store lock")
                .get(session_id)
                .cloned()
                .unwrap_or(Value::Null),
        };
        if value.is_null() {
            return Ok(SessionState::empty(session_id));
        }
        let state: SessionState = serde_json::from_value(value)
            .map_err(|error| format!("stored A2UI session `{session_id}` is malformed: {error}"))?;
        if state.session_id != session_id {
            return Err(format!(
                "stored A2UI session key `{session_id}` contains `{}`",
                state.session_id
            ));
        }
        Ok(state)
    }

    pub async fn save(&self, state: &SessionState) -> Result<(), String> {
        let value =
            serde_json::to_value(state).map_err(|error| format!("session serialize: {error}"))?;
        match &self.backend {
            Backend::Bus(iii) => Self::bus_call(
                iii,
                "state::set",
                json!({"scope": STATE_SCOPE, "key": state.session_id, "value": value}),
            )
            .await
            .map(|_| ()),
            Backend::Memory(values) => {
                values
                    .lock()
                    .expect("store lock")
                    .insert(state.session_id.clone(), value);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_session_is_empty_and_saved_state_round_trips() {
        let store = Store::in_memory();
        let mut state = store.load("s1").await.unwrap();
        assert_eq!(state.session_id, "s1");
        assert!(state.surfaces.is_empty());
        state.updated_at_ms = 42;
        store.save(&state).await.unwrap();
        assert_eq!(store.load("s1").await.unwrap().updated_at_ms, 42);
    }

    #[tokio::test]
    async fn mutation_locks_are_scoped_per_session() {
        let store = Store::in_memory();
        let first = store.mutation_guard("s1").await;
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            store.mutation_guard("s2"),
        )
        .await
        .expect("an unrelated session must not wait");
        drop(first);
        drop(second);
    }
}
