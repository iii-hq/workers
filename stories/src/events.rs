//! The two event functions this worker hosts. Both are private: only this
//! worker calls them, and their handlers return the event they are called
//! with, which is what observers observe.
//!
//! - `stories::on-changed` is called after every working-tree rebuild that
//!   changed at least one component, with the classified changes against
//!   the previous build. Args `{ workspace?, project?, kinds? }`; its deliver
//!   hook narrows the event to each observer's args and calls the observer
//!   with what is left, skipping an observer left with nothing.
//! - `stories::on-build` is called on every build state transition (queued,
//!   running, done, failed) of any line. No args; observers filter on
//!   `input.workspace`.
//!
//! The emitters call a function only while it has observers.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{Call, CallAction, CallRequest, IIIClient, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Change, ChangeKind, ChangeSummary, LineInfo};

pub const ON_CHANGED_FN: &str = "stories::on-changed";
pub const ON_BUILD_FN: &str = "stories::on-build";

/// The args of a `stories::on-changed` observer: which changes it wants.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedArgs {
    /// Workspace slug. Omit for every workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Project name. Omit for every project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Change kinds to receive: direct, indirect, new, removed. Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
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

/// Registers `stories::on-changed` and `stories::on-build`. The deliver hook
/// is set by id before the registration, so it serves every call.
pub fn register(iii: &Arc<IIIClient>) {
    let engine = iii.clone();
    iii.on_observer_deliver_for(
        ON_CHANGED_FN,
        move |call: Call, observers: Vec<Observer>| {
            let engine = engine.clone();
            async move { deliver_changed(&engine, call, observers).await }
        },
    );
    iii.register_function(
        ON_CHANGED_FN,
        RegisterFunction::new(|event: ChangedEvent| Ok::<_, Error>(event))
            .description(
                "Called after a working-tree rebuild changed components, with each change \
                 classified as direct, indirect, new or removed. Args: { workspace?, project?, \
                 kinds? }. Each observer receives the ChangedEvent narrowed to its args, and \
                 none when nothing is left.",
            )
            .args_format::<ChangedArgs>()
            .private(),
    );
    iii.register_function(
        ON_BUILD_FN,
        RegisterFunction::new(|event: BuildEvent| Ok::<_, Error>(event))
            .description(
                "Called on every build state transition (queued, running, done, failed) of any \
                 line. No args; filter on input.workspace.",
            )
            .private(),
    );
}

/// `stories::on-changed`'s delivery: each observer whose filters match gets
/// the event narrowed to its args, fire-and-forget; an observer left with
/// nothing is skipped. The caller gets the event back.
async fn deliver_changed(
    iii: &IIIClient,
    call: Call,
    observers: Vec<Observer>,
) -> Result<Value, Error> {
    let event: ChangedEvent = serde_json::from_value(call.input.clone())
        .map_err(|error| Error::Handler(format!("invalid {ON_CHANGED_FN} input: {error}")))?;
    for obs in observers {
        if !obs.matches(&call) {
            continue;
        }
        let Some(narrowed) = narrow_changed(obs.args_value(), &event) else {
            continue;
        };
        let Ok(payload) = serde_json::to_value(narrowed) else {
            continue;
        };
        let request = CallRequest::observer(&obs, payload).action(CallAction::Void);
        if let Err(error) = iii.call(request).await {
            tracing::warn!(observer_id = obs.id(), error = %error, "stories::on-changed delivery failed");
        }
    }
    Ok(call.output.unwrap_or(call.input))
}

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key)
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

/// The event narrowed to what an observer's args ask for; `None` when nothing
/// is left to deliver.
pub fn narrow_changed(args: &Value, event: &ChangedEvent) -> Option<ChangedEvent> {
    let workspace = text(args, "workspace");
    if !workspace.is_empty() && workspace != event.workspace {
        return None;
    }
    let project = text(args, "project");
    let kinds: Vec<&str> = args
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

/// Calls `function_id` with `event`, fire-and-forget, when it has observers.
fn emit(iii: &Arc<IIIClient>, function_id: &'static str, event: impl Serialize) {
    if !iii.is_observed(function_id) {
        return;
    }
    let Ok(payload) = serde_json::to_value(event) else {
        return;
    };
    let iii = iii.clone();
    tokio::spawn(async move {
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = iii.call(request).await {
            tracing::warn!(function_id, error = %error, "stories event call failed");
        }
    });
}

pub fn emit_changed(iii: &Arc<IIIClient>, event: &ChangedEvent) {
    emit(iii, ON_CHANGED_FN, event);
}

pub fn emit_build(iii: &Arc<IIIClient>, event: &BuildEvent) {
    emit(iii, ON_BUILD_FN, event);
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
    fn changed_args_take_only_workspace_project_and_kinds() {
        let args: ChangedArgs =
            serde_json::from_value(json!({ "workspace": "ws", "kinds": ["direct"] })).unwrap();
        assert_eq!(args.workspace.as_deref(), Some("ws"));
        assert!(serde_json::from_value::<ChangedArgs>(json!({ "metadata": {} })).is_err());
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
