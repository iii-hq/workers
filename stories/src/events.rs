//! The two trigger types this worker provides.
//!
//! - `stories:changed` fires after every working-tree rebuild that changed
//!   at least one component, with the classified changes against the
//!   previous build. Config `{ workspace?, project?, kinds? }`.
//! - `stories:build` fires on every build state transition (queued, running,
//!   done, failed) for any line. Config `{ workspace? }`.
//!
//! Bindings live in an in-process map keyed by trigger id; delivery is
//! fire-and-forget in the binding's namespace with its metadata attached.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Change, ChangeKind, ChangeSummary, LineInfo};

pub const CHANGED_TRIGGER: &str = "stories:changed";
pub const BUILD_TRIGGER: &str = "stories:build";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {
    /// Workspace slug. Omit for every workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Project name. Omit for every project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Change kinds to fire for: direct, indirect, new, removed. Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// Metadata attached to every invocation this binding receives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct BuildConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub workspace: String,
    pub line: LineInfo,
    /// The line the changes were classified against.
    pub base: LineInfo,
    pub summary: ChangeSummary,
    /// Only the changed components (never `unchanged`).
    pub changes: Vec<Change>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildEvent {
    pub workspace: String,
    pub build_id: String,
    /// queued | running | done | failed
    pub status: String,
    pub line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub at: String,
}

pub type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;

#[derive(Clone, Default)]
pub struct Subscribers {
    pub changed: Bindings,
    pub build: Bindings,
}

struct BindingTable(Bindings);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED_TRIGGER,
            "Fires after a working-tree rebuild changed components, with each change classified as direct, indirect, new or removed. Config: { workspace?, project?, kinds?, metadata? }.",
            BindingTable(subscribers.changed.clone()),
        )
        .trigger_request_format::<ChangedConfig>()
        .call_request_format::<ChangedEvent>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            BUILD_TRIGGER,
            "Fires on every build state transition (queued, running, done, failed) of any line. Config: { workspace?, metadata? }.",
            BindingTable(subscribers.build.clone()),
        )
        .trigger_request_format::<BuildConfig>()
        .call_request_format::<BuildEvent>(),
    );
}

fn text<'a>(config: &'a Value, key: &str) -> &'a str {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
}

fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Direct => "direct",
        ChangeKind::Indirect => "indirect",
        ChangeKind::New => "new",
        ChangeKind::Removed => "removed",
        ChangeKind::Unchanged => "unchanged",
    }
}

/// The event narrowed to what the binding asked for; `None` when nothing is
/// left to deliver.
pub fn narrow_changed(config: &Value, event: &ChangedEvent) -> Option<ChangedEvent> {
    let workspace = text(config, "workspace");
    if !workspace.is_empty() && workspace != event.workspace {
        return None;
    }
    let project = text(config, "project");
    let kinds: Vec<&str> = config
        .get("kinds")
        .and_then(Value::as_array)
        .map(|k| {
            k.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let changes: Vec<Change> = event
        .changes
        .iter()
        .filter(|c| project.is_empty() || c.project == project)
        .filter(|c| kinds.is_empty() || kinds.contains(&kind_name(c.kind)))
        .cloned()
        .collect();
    if changes.is_empty() {
        return None;
    }
    Some(ChangedEvent {
        summary: crate::model::summarize(&changes),
        changes,
        ..event.clone()
    })
}

pub fn matches_build(config: &Value, event: &BuildEvent) -> bool {
    let workspace = text(config, "workspace");
    workspace.is_empty() || workspace == event.workspace
}

fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| binding.metadata.clone())
}

fn deliver(iii: Arc<IIIClient>, deliveries: Vec<(TriggerConfig, Value)>) {
    if deliveries.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for (binding, payload) in deliveries {
            let mut request: TriggerRequestWithMetadata = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            }
            .into();
            if let Some(metadata) = subscription_metadata(&binding) {
                request = request.metadata(metadata);
            }
            if let Some(namespace) = &binding.namespace {
                request = request.namespace(namespace.clone());
            }
            if let Err(error) = iii.trigger(request).await {
                tracing::warn!(function_id = binding.function_id, error = %error, "stories trigger delivery failed");
            }
        }
    });
}

pub fn emit_changed(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &ChangedEvent) {
    let deliveries: Vec<(TriggerConfig, Value)> = subscribers
        .changed
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter_map(|binding| {
            let narrowed = narrow_changed(&binding.config, event)?;
            serde_json::to_value(narrowed)
                .ok()
                .map(|payload| (binding.clone(), payload))
        })
        .collect();
    deliver(iii.clone(), deliveries);
}

pub fn emit_build(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &BuildEvent) {
    let Ok(payload) = serde_json::to_value(event) else {
        return;
    };
    let deliveries: Vec<(TriggerConfig, Value)> = subscribers
        .build
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|binding| matches_build(&binding.config, event))
        .map(|binding| (binding.clone(), payload.clone()))
        .collect();
    deliver(iii.clone(), deliveries);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineKind, summarize};
    use serde_json::json;

    fn change(project: &str, kind: ChangeKind) -> Change {
        Change {
            id: "x".into(),
            project: project.into(),
            title: "X".into(),
            file: "x".into(),
            kind,
            files: vec![],
            a_version: None,
            b_version: None,
            states: vec![],
        }
    }

    fn event() -> ChangedEvent {
        let line = LineInfo {
            key: "worktree".into(),
            kind: LineKind::Worktree,
            label: "working tree".into(),
            sha: None,
            dirty: None,
            built_at: "t".into(),
        };
        let changes = vec![
            change("app", ChangeKind::Direct),
            change("admin", ChangeKind::Indirect),
        ];
        ChangedEvent {
            workspace: "ws".into(),
            line: line.clone(),
            base: line,
            summary: summarize(&changes),
            changes,
            at: "t".into(),
        }
    }

    #[test]
    fn narrowing_filters_workspace_project_and_kinds() {
        assert!(narrow_changed(&json!({ "workspace": "other" }), &event()).is_none());
        let narrowed = narrow_changed(&json!({ "project": "admin" }), &event()).unwrap();
        assert_eq!(narrowed.changes.len(), 1);
        assert_eq!(narrowed.summary.indirect, 1);
        assert!(narrow_changed(&json!({ "kinds": ["new"] }), &event()).is_none());
        assert_eq!(
            narrow_changed(&json!({}), &event()).unwrap().changes.len(),
            2
        );
    }
}
