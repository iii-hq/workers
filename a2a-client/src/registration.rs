//! Register every remote skill from a `Session` as a local iii function.
//!
//! Each remote skill becomes `<namespace>.<session.name>::<skill.id>` and
//! invoking it locally fans the call out as a `message/send` to the remote
//! agent. The poll loop diffs the agent card on `--poll-interval` cadence and
//! adds/removes registrations as the remote roster changes.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use iii_sdk::{FunctionRef, RegisterFunctionMessage, III};
use iii_sdk::IIIError;
use serde_json::{json, Value};
use tokio::time::interval;

use crate::session::Session;
use crate::types::{AgentSkill, Part, Task, TaskState};

/// Tracks every skill we've registered so we can `unregister()` it later.
type SkillMap = DashMap<String, FunctionRef>;

/// Register every skill currently advertised by `session`. Returns the map of
/// skill-id → `FunctionRef` so the poll loop can mutate it as the card drifts.
pub async fn register_all(iii: &III, session: Arc<Session>, namespace: &str) -> Arc<SkillMap> {
    let map = Arc::new(SkillMap::new());

    let card = session.card.read().await.clone();
    for skill in &card.skills {
        register_one(iii, &session, namespace, skill, &map);
    }

    tracing::info!(
        agent = %session.name,
        registered = map.len(),
        "a2a-client: initial skill registration complete"
    );
    map
}

/// Spawn the cadence loop. Periodically re-fetch the card and reconcile.
pub fn spawn_poll_loop(
    iii: III,
    session: Arc<Session>,
    namespace: String,
    map: Arc<SkillMap>,
    poll_interval: Duration,
) {
    tokio::spawn(async move {
        let mut tick = interval(poll_interval);
        // Skip the immediate first tick — initial registration ran in
        // `register_all` already; poll only catches drift.
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = reconcile(&iii, &session, &namespace, &map).await {
                tracing::warn!(
                    agent = %session.name,
                    error = %e,
                    "a2a-client: reconcile failed (will retry)"
                );
            }
        }
    });
}

async fn reconcile(
    iii: &III,
    session: &Arc<Session>,
    namespace: &str,
    map: &Arc<SkillMap>,
) -> anyhow::Result<()> {
    let card = session.refresh_card().await?;

    let live_ids: std::collections::HashSet<String> =
        card.skills.iter().map(|s| s.id.clone()).collect();

    // Drop skills that vanished.
    let to_drop: Vec<String> = map
        .iter()
        .filter_map(|entry| {
            if !live_ids.contains(entry.key()) {
                Some(entry.key().clone())
            } else {
                None
            }
        })
        .collect();
    for skill_id in to_drop {
        if let Some((_, function_ref)) = map.remove(&skill_id) {
            tracing::info!(
                agent = %session.name,
                skill = %skill_id,
                "a2a-client: unregistering vanished remote skill"
            );
            function_ref.unregister();
        }
    }

    // Add newcomers.
    for skill in &card.skills {
        if !map.contains_key(&skill.id) {
            register_one(iii, session, namespace, skill, map);
        }
    }
    Ok(())
}

fn register_one(
    iii: &III,
    session: &Arc<Session>,
    namespace: &str,
    skill: &AgentSkill,
    map: &Arc<SkillMap>,
) {
    let function_id = format!("{}.{}::{}", namespace, session.name, skill.id);
    let metadata = json!({
        "a2a.remote.base_url": session.base_url,
        "a2a.remote.skill_id": skill.id,
        "a2a.remote.tags": skill.tags,
    });

    let session_for_handler = session.clone();
    let skill_id_for_handler = skill.id.clone();

    let function_ref = iii.register_function_with(
        RegisterFunctionMessage {
            id: function_id.clone(),
            description: Some(if skill.description.is_empty() {
                skill.name.clone()
            } else {
                skill.description.clone()
            }),
            request_format: None,
            response_format: None,
            metadata: Some(metadata),
            invocation: None,
        },
        move |input: Value| {
            let session = session_for_handler.clone();
            let skill_id = skill_id_for_handler.clone();
            async move { invoke_remote(session, skill_id, input).await }
        },
    );

    map.insert(skill.id.clone(), function_ref);
    tracing::debug!(
        agent = %session.name,
        local = %function_id,
        remote = %skill.id,
        "a2a-client: registered remote skill"
    );
}

async fn invoke_remote(
    session: Arc<Session>,
    skill_id: String,
    input: Value,
) -> Result<Value, IIIError> {
    let task: Task = session
        .send_message(&skill_id, input)
        .await
        .map_err(|e| IIIError::Runtime(e.to_string()))?;

    match &task.status.state {
        TaskState::Completed => Ok(extract_result(&task)),
        other => Err(IIIError::Runtime(format!(
            "Remote task ended in state {:?}: {}",
            other,
            extract_failure_text(&task),
        ))),
    }
}

/// Returns the first part of the first artifact. Multi-part artifacts and
/// multi-artifact tasks are truncated; sufficient for v0.1.0 since most
/// remote skills return a single data/text part.
fn extract_result(task: &Task) -> Value {
    let part = task
        .artifacts
        .as_ref()
        .and_then(|arts| arts.first())
        .and_then(|art| art.parts.first());
    match part {
        Some(p) => part_to_value(p),
        None => Value::Null,
    }
}

fn part_to_value(p: &Part) -> Value {
    if let Some(data) = &p.data {
        return data.clone();
    }
    if let Some(text) = &p.text {
        // The iii-a2a server stores function results as
        // `Part { text: Some(json_string), media_type: "application/json" }`.
        // Try to parse JSON so callers don't have to double-decode.
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            return parsed;
        }
        return json!({ "text": text });
    }
    Value::Null
}

fn extract_failure_text(task: &Task) -> String {
    let from_message = task
        .status
        .message
        .as_ref()
        .and_then(|m| m.parts.first())
        .and_then(|p| p.text.clone());
    if let Some(text) = from_message {
        if !text.is_empty() {
            return text;
        }
    }
    let timestamp = task.status.timestamp.as_deref().unwrap_or("unknown");
    format!("no failure message; task_id={} timestamp={}", task.id, timestamp)
}
