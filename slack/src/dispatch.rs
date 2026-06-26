//! Route inbound Slack events and interactions: gate harness turns on explicit
//! mention, capture untagged messages as thread context, and resolve approvals.

use crate::clients::{approval, slack};
use crate::deps::Deps;
use crate::kv::{self, PendingMsg};
use crate::turn;
use crate::types::{EventCallback, InteractionPayload, MessageEvent, SlackEvent};
use serde_json::json;

/// Returns true if this `event_id` is new (and records it).
fn first_time(deps: &Deps, event_id: Option<&str>) -> bool {
    let Some(id) = event_id else {
        return true;
    };
    deps.runtime
        .seen_events
        .insert(id.to_string(), ())
        .is_none()
}

pub async fn process_event(deps: &Deps, cb: EventCallback) {
    if !first_time(deps, cb.event_id.as_deref()) {
        return;
    }
    let team = cb.team_id.clone();
    match cb.event {
        SlackEvent::AppMention(m) => {
            handle_mention(deps, team.as_deref(), m).await;
        }
        SlackEvent::Message(m) => {
            handle_message(deps, team.as_deref(), m).await;
        }
        SlackEvent::AssistantThreadStarted { assistant_thread } => {
            tracing::info!(
                channel = %assistant_thread.channel_id,
                "assistant thread started"
            );
        }
        SlackEvent::Other => {}
    }
}

async fn is_self(deps: &Deps, user: Option<&str>) -> bool {
    match (user, deps.bot_user_id().await) {
        (Some(u), Some(bot)) => u == bot,
        _ => false,
    }
}

async fn handle_mention(deps: &Deps, team: Option<&str>, m: MessageEvent) {
    if m.bot_id.is_some() || is_self(deps, m.user.as_deref()).await {
        return;
    }
    let (Some(channel), Some(ts)) = (m.channel.clone(), m.ts.clone()) else {
        return;
    };
    let thread_ts = m.thread_ts.clone().unwrap_or(ts.clone());
    let text = m.text.clone().unwrap_or_default();
    if let Err(e) = turn::handle_addressed(
        deps,
        team,
        &channel,
        &thread_ts,
        m.user.as_deref(),
        &text,
        &ts,
    )
    .await
    {
        tracing::warn!(error = %e, "handle_addressed (mention) failed");
    }
}

async fn handle_message(deps: &Deps, team: Option<&str>, m: MessageEvent) {
    // Ignore bot/self messages and edits/joins/etc. (any subtype).
    if m.bot_id.is_some() || m.subtype.is_some() || is_self(deps, m.user.as_deref()).await {
        return;
    }
    let Some(channel) = m.channel.clone() else {
        return;
    };
    let ts = m.ts.clone().unwrap_or_default();
    let thread_ts = m.thread_ts.clone().unwrap_or_else(|| ts.clone());
    let text = m.text.clone().unwrap_or_default();

    // DMs are implicitly addressed to the bot → trigger.
    if m.channel_type.as_deref() == Some("im") {
        if let Err(e) = turn::handle_addressed(
            deps,
            team,
            &channel,
            &thread_ts,
            m.user.as_deref(),
            &text,
            &ts,
        )
        .await
        {
            tracing::warn!(error = %e, "handle_addressed (im) failed");
        }
        return;
    }

    // Channel message. If it mentions the bot, the app_mention event will drive
    // the turn — don't double-handle here. Otherwise capture it as context.
    if mentions_bot(deps, &text).await {
        return;
    }
    let _ = kv::push_pending(
        deps,
        team,
        &channel,
        &thread_ts,
        PendingMsg {
            user: m.user.clone(),
            text,
        },
        deps.cfg().await.backfill_max_messages,
    )
    .await;
}

async fn mentions_bot(deps: &Deps, text: &str) -> bool {
    match deps.bot_user_id().await {
        Some(id) => text.contains(&format!("<@{id}>")),
        None => false,
    }
}

pub async fn process_interaction(deps: &Deps, payload: InteractionPayload) {
    if payload.kind != "block_actions" {
        return;
    }
    let cfg = deps.cfg().await;
    for action in &payload.actions {
        let Some(token) = action.value.clone() else {
            continue;
        };
        let Some(cb) = kv::load_approval_cb(deps, &token).await else {
            continue;
        };
        let result = match action.action_id.as_str() {
            "slack_approve" => {
                approval::resolve(
                    &deps.iii,
                    &cb.session_id,
                    &cb.function_call_id,
                    true,
                    cfg.timeout_ms,
                )
                .await
            }
            "slack_reject" => {
                approval::resolve(
                    &deps.iii,
                    &cb.session_id,
                    &cb.function_call_id,
                    false,
                    cfg.timeout_ms,
                )
                .await
            }
            "slack_approve_always" => {
                let _ = approval::resolve(
                    &deps.iii,
                    &cb.session_id,
                    &cb.function_call_id,
                    true,
                    cfg.timeout_ms,
                )
                .await;
                approval::approve_always(&deps.iii, &cb.session_id, &cb.function_id, cfg.timeout_ms)
                    .await
            }
            _ => continue,
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "approval resolve failed");
        }
        // Clear the buttons.
        let _ = slack::call(
            deps,
            "chat.update",
            json!({
                "channel": cb.channel,
                "ts": cb.message_ts,
                "text": format!("Approval resolved: {}", action.action_id.trim_start_matches("slack_")),
                "blocks": [],
            }),
        )
        .await;
        kv::delete_approval_cb(deps, &token).await;
    }
}
