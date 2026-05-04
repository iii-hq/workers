//! Custom trigger types this worker publishes.
//!
//! Two trigger types exist:
//!
//! - `skills::on-change`  — fires once per mutation of the skills scope.
//! - `prompts::on-change` — fires once per mutation of the prompts scope.
//!
//! The `mcp` worker (and any other interested subscriber) registers a
//! trigger instance of these types via
//! `iii.register_trigger(RegisterTriggerInput { trigger_type: "skills::on-change", ... })`.
//! The engine routes that registration through our [`SkillsTriggerHandler`]
//! which stashes the subscriber in [`SubscriberSet`]. When a state
//! mutation lands, the owning function module (`functions::skills` /
//! `functions::prompts`) reads the active subscribers and invokes each
//! one via `iii.trigger` — a simple in-process fanout.
//!
//! We intentionally don't rely on the engine's built-in `state` trigger
//! to dispatch directly to mcp because that would hard-code the
//! `mcp::*` function ids inside skills. Using a named custom trigger
//! keeps the coupling one-way: mcp knows skills publishes
//! `skills::on-change`; skills never has to know mcp exists.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::{
    IIIError, RegisterTriggerType, TriggerAction, TriggerConfig, TriggerHandler, TriggerRequest,
    III,
};
use serde_json::Value;

pub const SKILLS_ON_CHANGE: &str = "skills::on-change";
pub const PROMPTS_ON_CHANGE: &str = "prompts::on-change";

/// Thread-safe subscriber registry keyed by trigger-instance id. Cloned
/// into both the `TriggerHandler` (which mutates on register /
/// unregister) and the fan-out path in the function modules (which
/// iterates read-only). Entries are `TriggerConfig` so the fan-out can
/// see the subscriber's `function_id`.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, TriggerConfig>>>,
}

impl SubscriberSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, config: TriggerConfig) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(config.id.clone(), config);
    }

    fn remove(&self, id: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(id);
    }

    /// Snapshot of the current subscribers as a Vec of `function_id`s.
    /// Returns a snapshot so the mutex isn't held across awaits.
    pub fn function_ids(&self) -> Vec<String> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values().map(|c| c.function_id.clone()).collect()
    }
}

/// Fire `payload` to every subscriber using `TriggerAction::Void`
/// (fire-and-forget so the mutation that produced the change isn't
/// blocked on downstream latency). Failures are logged and swallowed
/// because a slow / misbehaving subscriber must not break the write
/// path.
pub async fn dispatch(iii: &III, subscribers: &SubscriberSet, payload: Value) {
    let targets = subscribers.function_ids();
    for function_id in targets {
        let fid = function_id.clone();
        let payload_copy = payload.clone();
        let res = iii
            .trigger(TriggerRequest {
                function_id: fid,
                payload: payload_copy,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(
                function_id = %function_id,
                error = %e,
                "on-change fan-out failed"
            );
        }
    }
}

pub struct RegisteredTriggerTypes {
    pub skills: SubscriberSet,
    pub prompts: SubscriberSet,
}

pub fn register_all(iii: &Arc<III>) -> RegisteredTriggerTypes {
    let skills = SubscriberSet::new();
    let prompts = SubscriberSet::new();

    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        SKILLS_ON_CHANGE.to_string(),
        "Fires after any mutation of the skills registry (register / unregister).".to_string(),
        SkillsTriggerHandler::new(SKILLS_ON_CHANGE, skills.clone()),
    ));
    tracing::info!(trigger_type = SKILLS_ON_CHANGE, "registered trigger type");

    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        PROMPTS_ON_CHANGE.to_string(),
        "Fires after any mutation of the prompts registry (register / unregister).".to_string(),
        SkillsTriggerHandler::new(PROMPTS_ON_CHANGE, prompts.clone()),
    ));
    tracing::info!(trigger_type = PROMPTS_ON_CHANGE, "registered trigger type");

    RegisteredTriggerTypes { skills, prompts }
}

struct SkillsTriggerHandler {
    name: String,
    subscribers: SubscriberSet,
}

impl SkillsTriggerHandler {
    fn new(name: &str, subscribers: SubscriberSet) -> Self {
        Self {
            name: name.into(),
            subscribers,
        }
    }
}

#[async_trait]
impl TriggerHandler for SkillsTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_config(id: &str, function_id: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config: json!({}),
            metadata: None,
        }
    }

    #[test]
    fn subscriber_set_insert_and_remove() {
        let set = SubscriberSet::new();
        assert!(set.function_ids().is_empty());
        set.insert(make_config("sub-1", "mcp::__on_skills_changed"));
        set.insert(make_config("sub-2", "other::receiver"));
        let mut fns = set.function_ids();
        fns.sort();
        assert_eq!(
            fns,
            vec![
                "mcp::__on_skills_changed".to_string(),
                "other::receiver".to_string()
            ]
        );
        set.remove("sub-1");
        assert_eq!(set.function_ids(), vec!["other::receiver".to_string()]);
    }

    #[test]
    fn subscriber_set_duplicate_id_overwrites() {
        let set = SubscriberSet::new();
        set.insert(make_config("sub-1", "a"));
        set.insert(make_config("sub-1", "b"));
        assert_eq!(set.function_ids(), vec!["b".to_string()]);
    }
}
