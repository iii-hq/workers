//! Custom trigger types this worker publishes.
//!
//! Three trigger types exist:
//!
//! - `directory::skills::on-change`  — fires after every successful
//!   `directory::skills::download` that wrote at least one skill
//!   markdown file, or a `directory::skills::update`, `create`, or
//!   `delete`. External edits under the read-only `agents_skills_folder`
//!   also fire it via the fs watcher (doorbell only — the worker never
//!   writes there).
//! - `directory::system-prompts::on-change` — fires after every
//!   successful `directory::skills::download` that wrote at least one
//!   system prompt, a `directory::system-prompts::update`, or a
//!   `directory::system-prompts::create` / `delete`.
//! - `directory::agents::on-change` — fires after every successful
//!   download or direct create, update, delete, or external edit of an
//!   agent profile.
//!
//! The `mcp` worker (and any other interested subscriber) registers a
//! trigger instance of these types via
//! `iii.register_trigger(RegisterTriggerInput::new("directory::skills::on-change", ...))`.
//! The engine routes that registration through our
//! [`SkillsTriggerHandler`] which stashes the subscriber in
//! [`SubscriberSet`]. When a download lands, the `functions::download`
//! module reads the active subscribers and invokes each one via
//! `iii.trigger` — a simple in-process fanout.
//!
//! Using a named custom trigger keeps the coupling one-way: mcp knows
//! the directory worker publishes `directory::skills::on-change`; the
//! directory worker never has to know mcp exists.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use serde_json::Value;

pub const SKILLS_ON_CHANGE: &str = "directory::skills::on-change";
pub const SYSTEM_PROMPTS_ON_CHANGE: &str = "directory::system-prompts::on-change";
pub const AGENTS_ON_CHANGE: &str = "directory::agents::on-change";

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

    /// Snapshot of the current subscribers and their target namespaces.
    /// Returns a snapshot so the mutex isn't held across awaits.
    pub fn targets(&self) -> Vec<(String, Option<String>)> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values()
            .map(|c| (c.function_id.clone(), c.namespace.clone()))
            .collect()
    }
}

/// Fire `payload` to every subscriber using `TriggerAction::Void`
/// (fire-and-forget so the mutation that produced the change isn't
/// blocked on downstream latency). Failures are logged and swallowed
/// because a slow / misbehaving subscriber must not break the write
/// path.
pub async fn dispatch(iii: &IIIClient, subscribers: &SubscriberSet, payload: Value) {
    let targets = subscribers.targets();
    for (function_id, namespace) in targets {
        let fid = function_id.clone();
        let payload_copy = payload.clone();
        let request = TriggerRequest {
            function_id: fid,
            payload: payload_copy,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        };
        let namespace = namespace.as_deref().unwrap_or("default");
        let res = iii.trigger(request.namespace(namespace)).await;
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
    pub system_prompts: SubscriberSet,
    pub agents: SubscriberSet,
}

pub fn register_all(iii: &Arc<IIIClient>) -> RegisteredTriggerTypes {
    let skills = SubscriberSet::new();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        SKILLS_ON_CHANGE.to_string(),
        "Fires after a directory::skills::download that wrote at least one skill markdown file, \
         or a directory::skills::update, create, or delete. Also fires with { op: \"external\" } \
         when a watched skills root changes on disk outside this worker — including the \
         read-only agents skills root, when that root exists at startup."
            .to_string(),
        SkillsTriggerHandler::new(SKILLS_ON_CHANGE, skills.clone()),
    ));
    tracing::info!(trigger_type = SKILLS_ON_CHANGE, "registered trigger type");

    let system_prompts = SubscriberSet::new();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        SYSTEM_PROMPTS_ON_CHANGE.to_string(),
        "Fires after a directory::skills::download that wrote at least one system prompt, or a \
         directory::system-prompts::update, create, or delete. Also fires with \
         { op: \"external\" } when a watched system-prompts root changes on disk outside \
         this worker."
            .to_string(),
        SkillsTriggerHandler::new(SYSTEM_PROMPTS_ON_CHANGE, system_prompts.clone()),
    ));
    tracing::info!(
        trigger_type = SYSTEM_PROMPTS_ON_CHANGE,
        "registered trigger type"
    );

    let agents = SubscriberSet::new();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        AGENTS_ON_CHANGE.to_string(),
        "Fires after a directory::skills::download that wrote at least one agent profile, \
         or a directory::agents::update, create, or delete. Also fires with \
         { op: \"external\" } when a watched agents file changes on disk outside this \
         worker."
            .to_string(),
        SkillsTriggerHandler::new(AGENTS_ON_CHANGE, agents.clone()),
    ));
    tracing::info!(trigger_type = AGENTS_ON_CHANGE, "registered trigger type");

    RegisteredTriggerTypes {
        skills,
        system_prompts,
        agents,
    }
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
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
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
            namespace: None,
        }
    }

    #[test]
    fn subscriber_set_insert_and_remove() {
        let set = SubscriberSet::new();
        assert!(set.targets().is_empty());
        let mut namespaced = make_config("sub-1", "mcp::__on_skills_changed");
        namespaced.namespace = Some("project-a".into());
        set.insert(namespaced);
        set.insert(make_config("sub-2", "other::receiver"));
        let mut targets = set.targets();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                (
                    "mcp::__on_skills_changed".to_string(),
                    Some("project-a".to_string())
                ),
                ("other::receiver".to_string(), None)
            ]
        );
        set.remove("sub-1");
        assert_eq!(set.targets(), vec![("other::receiver".to_string(), None)]);
    }

    #[test]
    fn subscriber_set_duplicate_id_overwrites() {
        let set = SubscriberSet::new();
        set.insert(make_config("sub-1", "a"));
        set.insert(make_config("sub-1", "b"));
        assert_eq!(set.targets(), vec![("b".to_string(), None)]);
    }
}
