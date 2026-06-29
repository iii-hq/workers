//! Bridge bindings: drive Slack streaming and approvals from engine events
//! (`session::*`, `harness::turn-completed`, `approval::*`).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

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
            if evt.status != "completed" && evt.status != "ok" {
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
        "Observe approval resolution (button click already cleared the prompt).",
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
    if !deps.approval_gate.is_available() {
        return Ok(());
    }
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

/// Bind the session/harness triggers (best-effort; siblings may be absent).
pub fn bind_session_triggers(iii: &Arc<IIIClient>) {
    let bindings = [
        (
            "session::message-added",
            ON_MESSAGE_ADDED_ID,
            json!({ "roles": ["assistant"] }),
        ),
        (
            "session::message-updated",
            ON_MESSAGE_UPDATED_ID,
            json!({ "roles": ["assistant"] }),
        ),
        ("harness::turn-completed", ON_TURN_COMPLETED_ID, json!({})),
    ];
    for (trigger_type, function_id, config) in bindings {
        bind_best_effort(iii, trigger_type, function_id, config);
    }
}

/// Bind the approval triggers (only meaningful when `approval-gate` is present).
pub fn bind_approval_triggers(iii: &Arc<IIIClient>) {
    let bindings = [
        (
            "approval::pending-created",
            ON_PENDING_CREATED_ID,
            json!({}),
        ),
        (
            "approval::pending-resolved",
            ON_PENDING_RESOLVED_ID,
            json!({}),
        ),
    ];
    for (trigger_type, function_id, config) in bindings {
        bind_best_effort(iii, trigger_type, function_id, config);
    }
}

fn bind_best_effort(
    iii: &Arc<IIIClient>,
    trigger_type: &str,
    function_id: &str,
    config: serde_json::Value,
) {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    }) {
        Ok(_) => tracing::info!(trigger_type, function_id, "trigger bound"),
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger bind failed (sibling absent?)")
        }
    }
}
