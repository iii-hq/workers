//! Bridge observers: drive Slack streaming and approvals from the events of
//! session-manager (`session::on-message-*`), the harness
//! (`harness::on-turn-completed`) and approval-gate (`approval::on-pending-*`).
//! Each target receives the event itself (`deliver: output`). The observers
//! are created at boot and stay `pending` while their worker is absent; an
//! `approval::on-pending-created` event comes from a connected approval-gate,
//! so the prompt it posts can be answered.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{Deliver, IIIClient, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::slack;
use crate::deps::Deps;
use crate::kv;
use crate::types::{
    MessageAddedEvent, MessageUpdatedEvent, PendingApprovalRecord, PendingResolvedEvent,
    TurnCompletedEvent,
};
use crate::{stream, types::ApprovalCallback};

pub const ON_MESSAGE_ADDED_ID: &str = "slack::on-message-added";
pub const ON_MESSAGE_UPDATED_ID: &str = "slack::on-message-updated";
pub const ON_TURN_COMPLETED_ID: &str = "slack::on-turn-completed";
pub const ON_PENDING_CREATED_ID: &str = "slack::on-pending-created";
pub const ON_PENDING_RESOLVED_ID: &str = "slack::on-pending-resolved";

/// The observed functions of session-manager, the harness and approval-gate.
pub const SESSION_MESSAGE_ADDED_FN: &str = "session::on-message-added";
pub const SESSION_MESSAGE_UPDATED_FN: &str = "session::on-message-updated";
pub const HARNESS_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
pub const APPROVAL_PENDING_CREATED_FN: &str = "approval::on-pending-created";
pub const APPROVAL_PENDING_RESOLVED_FN: &str = "approval::on-pending-resolved";

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct BindingAck {
    pub ok: bool,
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        ON_MESSAGE_ADDED_ID,
        "Seed the Slack stream for a new assistant entry.",
        |d, evt: MessageAddedEvent| async move {
            if let Some(msg) = &evt.message {
                if msg.role == "assistant" {
                    let text = msg.text();
                    if let Err(e) = stream::on_assistant_text(&d, &evt.session_id, &text, 0).await {
                        tracing::warn!(error = %e, "on-message-added stream failed");
                    }
                }
            }
            Ok::<_, Error>(BindingAck { ok: true })
        },
    );

    super::register(
        iii,
        deps,
        ON_MESSAGE_UPDATED_ID,
        "Stream assistant text revisions into Slack.",
        |d, evt: MessageUpdatedEvent| async move {
            if evt.message.role == "assistant" {
                let text = evt.message.text();
                if let Err(e) =
                    stream::on_assistant_text(&d, &evt.session_id, &text, evt.revision).await
                {
                    tracing::warn!(error = %e, "on-message-updated stream failed");
                }
            }
            Ok::<_, Error>(BindingAck { ok: true })
        },
    );

    super::register(
        iii,
        deps,
        ON_TURN_COMPLETED_ID,
        "Finalize the Slack stream and report turn errors.",
        |d, evt: TurnCompletedEvent| async move {
            if let Err(e) = stream::finalize(&d, &evt.session_id).await {
                tracing::warn!(error = %e, "finalize stream failed");
            }
            // Non-terminal completion (armed wake): the run continues in a
            // later turn — flush the stream above but hold outcome reporting
            // for the terminal turn.
            if evt.terminal && evt.status != "completed" && evt.status != "ok" {
                if let Some(target) = kv::session_target(&d, &evt.session_id).await {
                    let reason = evt
                        .result_error
                        .or(evt.reason)
                        .unwrap_or_else(|| evt.status.clone());
                    let _ = slack::call(
                        &d,
                        "chat.postMessage",
                        json!({ "channel": target.channel, "thread_ts": target.thread_ts,
                            "text": format!("Turn ended: {reason}") }),
                    )
                    .await;
                }
            }
            Ok::<_, Error>(BindingAck { ok: true })
        },
    );

    super::register(
        iii,
        deps,
        ON_PENDING_CREATED_ID,
        "Post an inline Block Kit approval when a tool call is held.",
        |d, rec: PendingApprovalRecord| async move {
            if let Err(e) = post_approval(&d, &rec).await {
                tracing::warn!(error = %e, "post approval failed");
            }
            Ok::<_, Error>(BindingAck { ok: true })
        },
    );

    super::register(
        iii,
        deps,
        ON_PENDING_RESOLVED_ID,
        "Log approval resolution (button click already cleared the prompt).",
        |_d, evt: PendingResolvedEvent| async move {
            tracing::info!(
                session = %evt.session_id,
                call = %evt.function_call_id,
                "approval resolved"
            );
            Ok::<_, Error>(BindingAck { ok: true })
        },
    );
}

async fn post_approval(deps: &Deps, rec: &PendingApprovalRecord) -> Result<(), Error> {
    let Some(target) = kv::session_target(deps, &rec.session_id).await else {
        return Ok(());
    };
    let token = &rec.function_call_id;
    let blocks = json!([
        { "type": "section", "text": { "type": "mrkdwn",
            "text": format!("*Approve tool call* `{}`?", rec.function_id) } },
        { "type": "actions", "elements": [
            { "type": "button", "text": { "type": "plain_text", "text": "Approve" },
              "style": "primary", "action_id": "slack_approve", "value": token },
            { "type": "button", "text": { "type": "plain_text", "text": "Reject" },
              "style": "danger", "action_id": "slack_reject", "value": token },
            { "type": "button", "text": { "type": "plain_text", "text": "Approve always" },
              "action_id": "slack_approve_always", "value": token },
        ]},
    ]);
    let resp = slack::call(
        deps,
        "chat.postMessage",
        json!({ "channel": target.channel, "thread_ts": target.thread_ts,
            "text": format!("Approve tool call {}?", rec.function_id), "blocks": blocks }),
    )
    .await?;
    let message_ts = resp
        .get("ts")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let cb = ApprovalCallback {
        session_id: rec.session_id.clone(),
        function_call_id: rec.function_call_id.clone(),
        function_id: rec.function_id.clone(),
        channel: target.channel,
        message_ts,
    };
    kv::store_approval_cb(deps, token, &cb).await
}

/// Assistant messages only.
fn assistant_messages() -> Value {
    json!({ "input.message.role": { "$in": ["assistant"] } })
}

/// `(source, target, filters)` of the session and harness observers.
pub fn session_observers() -> [(&'static str, &'static str, Option<Value>); 3] {
    [
        (
            SESSION_MESSAGE_ADDED_FN,
            ON_MESSAGE_ADDED_ID,
            Some(assistant_messages()),
        ),
        (
            SESSION_MESSAGE_UPDATED_FN,
            ON_MESSAGE_UPDATED_ID,
            Some(assistant_messages()),
        ),
        (HARNESS_TURN_COMPLETED_FN, ON_TURN_COMPLETED_ID, None),
    ]
}

/// `(source, target, filters)` of the approval observers.
pub fn approval_observers() -> [(&'static str, &'static str, Option<Value>); 2] {
    [
        (APPROVAL_PENDING_CREATED_FN, ON_PENDING_CREATED_ID, None),
        (APPROVAL_PENDING_RESOLVED_FN, ON_PENDING_RESOLVED_ID, None),
    ]
}

/// `(source, target, filters)` of every observer [`observe_events`] creates.
pub fn boot_observers() -> Vec<(&'static str, &'static str, Option<Value>)> {
    session_observers()
        .into_iter()
        .chain(approval_observers())
        .collect()
}

/// Observe the session, harness and approval events. Each observer stays
/// `pending` while its worker is absent and becomes active when it registers.
pub fn observe_events(iii: &Arc<IIIClient>) {
    for (source, target, filters) in boot_observers() {
        observe(iii, source, target, filters);
    }
}

/// The observer lives as long as the worker's connection; dropping the
/// handle does not dispose it.
fn observe(iii: &Arc<IIIClient>, source: &str, target: &str, filters: Option<Value>) {
    let mut src = Source::function(source).deliver(Deliver::Output);
    if let Some(filters) = filters {
        src = src.filters(filters);
    }
    let handle = iii.observe(src, target);
    tracing::info!(
        function_id = source,
        observer_target = target,
        observer_id = handle.id(),
        "observing"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_observers_select_assistant_messages() {
        for (source, _, filters) in session_observers() {
            if source.starts_with("session::") {
                assert_eq!(
                    filters,
                    Some(json!({ "input.message.role": { "$in": ["assistant"] } }))
                );
            } else {
                assert_eq!(filters, None);
            }
        }
    }

    #[test]
    fn boot_observes_the_session_harness_and_approval_events() {
        let sources: Vec<_> = boot_observers()
            .into_iter()
            .map(|(source, target, _)| (source, target))
            .collect();
        assert_eq!(
            sources,
            vec![
                ("session::on-message-added", "slack::on-message-added"),
                ("session::on-message-updated", "slack::on-message-updated"),
                ("harness::on-turn-completed", "slack::on-turn-completed"),
                ("approval::on-pending-created", "slack::on-pending-created"),
                (
                    "approval::on-pending-resolved",
                    "slack::on-pending-resolved"
                ),
            ]
        );
    }
}
