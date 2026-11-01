//! `secrets::on-changed`: a private function whose handler returns the change
//! event it is called with.
//!
//! The worker calls it after every create, rotation, deletion and allowlist
//! change with metadata only, never a value; for the env store, also when an
//! edit to `.env` sets, changes or removes a shared variable. It takes no
//! args: observers select secrets with filters on `input.name` (a bare name,
//! both stores) or `input.ref` (`secret://NAME`, `env://NAME`). iii delivers
//! each call to every observer whose filters match it, fire-and-forget.
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::api::ids::ON_CHANGED;
use crate::api::StoreKind;
use crate::names::{reference_for, split_reference};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Created,
    Rotated,
    Deleted,
    AccessChanged,
}

/// A `secrets::on-changed` event. Never carries a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub name: String,
    /// `secret://NAME` or `env://NAME`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub action: ChangeAction,
    /// The current fingerprint; absent after a deletion and for the env store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub updated_at: String,
}

impl ChangedEvent {
    /// A vault secret's event.
    pub fn new(
        name: &str,
        action: ChangeAction,
        fingerprint: Option<String>,
        updated_at: &str,
    ) -> Self {
        Self {
            name: name.to_owned(),
            reference: reference_for(StoreKind::Vault, name),
            action,
            fingerprint,
            updated_at: updated_at.to_owned(),
        }
    }

    /// A shared environment variable's event.
    pub fn env(name: &str, action: ChangeAction, updated_at: &str) -> Self {
        Self {
            name: name.to_owned(),
            reference: reference_for(StoreKind::Env, name),
            action,
            fingerprint: None,
            updated_at: updated_at.to_owned(),
        }
    }

    /// The store the event is about, from its reference.
    pub fn store(&self) -> StoreKind {
        split_reference(&self.reference)
            .0
            .unwrap_or(StoreKind::Vault)
    }
}

pub const ON_CHANGED_DESCRIPTION: &str = "Called after a secret is created, rotated, deleted or its consumers change, and after an edit to .env changes a shared variable; returns the event: name, ref, action, fingerprint and updated_at, never the value. Observe it with filters on input.name (both stores) or input.ref (secret://NAME, env://NAME). Only the secrets worker calls it.";

/// Registers `secrets::on-changed`: private (only this worker calls it) and
/// observable by any worker.
pub fn register(iii: &IIIClient) {
    iii.register_function(
        ON_CHANGED,
        RegisterFunction::new(|event: ChangedEvent| Ok::<_, Error>(event))
            .description(ON_CHANGED_DESCRIPTION)
            .private(),
    );
}

/// Calls `secrets::on-changed` with `event`, fire-and-forget, when anything
/// observes it.
pub fn emit(iii: &Arc<IIIClient>, event: &ChangedEvent) {
    if !iii.is_observed(ON_CHANGED) {
        return;
    }
    let payload = match serde_json::to_value(event) {
        Ok(payload) => payload,
        Err(_) => return,
    };
    let iii = iii.clone();
    tokio::spawn(async move {
        let request = CallRequest {
            action: Some(CallAction::Void),
            ..CallRequest::new(ON_CHANGED, payload)
        };
        if let Err(error) = iii.call(request).await {
            tracing::warn!(error = %error, "secrets::on-changed call failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_shape_never_has_a_value() {
        let event = ChangedEvent::new("A", ChangeAction::AccessChanged, Some("00ff".into()), "t");
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({"name":"A","ref":"secret://A","action":"access_changed","fingerprint":"00ff","updated_at":"t"})
        );
        let deleted = ChangedEvent::new("A", ChangeAction::Deleted, None, "t");
        assert!(serde_json::to_value(deleted)
            .unwrap()
            .get("fingerprint")
            .is_none());
        let env = ChangedEvent::env("A", ChangeAction::Rotated, "t");
        assert_eq!(env.store(), StoreKind::Env);
        assert_eq!(
            serde_json::to_value(&env).unwrap(),
            json!({"name":"A","ref":"env://A","action":"rotated","updated_at":"t"})
        );
    }
}
