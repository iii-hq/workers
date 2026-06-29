//! Turn orchestration: when the bot is addressed, backfill thread context,
//! resolve a model, and start a `harness::send` turn bound to the Slack thread.

use iii_sdk::errors::Error;
use serde_json::{json, Value};

use crate::clients::{harness, router, slack};
use crate::config::{ModelRef, WorkerConfig};
use crate::deps::Deps;
use crate::kv::{self, PendingMsg};

const CHANNEL_CONTEXT: &str = include_str!("../prompts/channel-context.txt");

/// Entry point when the bot is explicitly addressed in a thread.
#[allow(clippy::too_many_arguments)]
pub async fn handle_addressed(
    deps: &Deps,
    team: Option<&str>,
    channel: &str,
    thread_ts: &str,
    user_id: Option<&str>,
    raw_text: &str,
    event_ts: &str,
) -> Result<(), Error> {
    let cfg = deps.cfg().await;

    if !allowed(&cfg, channel, team) {
        tracing::info!(channel, "message ignored (not in allowlist)");
        return Ok(());
    }

    let session = kv::thread_session(deps, team, channel, thread_ts).await;
    let is_new = session.is_none();

    // Build context from captured (untagged) messages, plus the full thread on
    // the first mention.
    let mut context: Vec<PendingMsg> = Vec::new();
    if is_new && cfg.backfill_thread {
        context.extend(fetch_thread(deps, channel, thread_ts, cfg.backfill_max_messages).await);
    }
    context.extend(kv::take_pending(deps, team, channel, thread_ts).await);
    truncate_front(&mut context, cfg.backfill_max_messages as usize);

    let user_text = strip_mention(raw_text, deps.bot_user_id().await.as_deref());
    let message = compose_message(&context, &user_text);

    let Some(model) = resolve_model(deps, &cfg).await else {
        let _ = slack::call(
            deps,
            "chat.postMessage",
            json!({ "channel": channel, "thread_ts": thread_ts,
                "text": "No model is available. Connect a provider (llm-router) or set default_model." }),
        )
        .await;
        return Ok(());
    };

    let system_prompt = build_system_prompt(&cfg, channel, thread_ts, session.as_deref());
    let options = harness::SendOptions {
        system_prompt: Some(system_prompt),
        functions: Some(harness::FunctionsPolicy {
            allow: cfg.functions_allow.clone(),
        }),
        metadata: Some(harness::metadata(channel, thread_ts, &model)),
    };

    let req = harness::SendRequest {
        session_id: session.clone(),
        message,
        model: model.id.clone(),
        provider: model.provider.clone(),
        idempotency_key: Some(format!("slack-{channel}-{event_ts}")),
        options: Some(options),
    };

    let resp = match harness::send(&deps.iii, req, cfg.timeout_ms).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "harness::send failed");
            let _ = slack::call(
                deps,
                "chat.postMessage",
                json!({ "channel": channel, "thread_ts": thread_ts,
                    "text": format!("Could not start a turn: {e}") }),
            )
            .await;
            return Ok(());
        }
    };

    kv::set_thread_session(deps, team, channel, thread_ts, &resp.session_id, user_id).await?;

    // Best-effort "thinking" indicator (valid only in assistant threads).
    let _ = slack::call(
        deps,
        "assistant.threads.setStatus",
        json!({ "channel_id": channel, "thread_ts": thread_ts, "status": "is thinking…" }),
    )
    .await;

    Ok(())
}

fn allowed(cfg: &WorkerConfig, channel: &str, team: Option<&str>) -> bool {
    if !cfg.allowed_channels.is_empty() && !cfg.allowed_channels.iter().any(|c| c == channel) {
        return false;
    }
    if !cfg.allowed_teams.is_empty() {
        match team {
            Some(t) if cfg.allowed_teams.iter().any(|x| x == t) => {}
            _ => return false,
        }
    }
    true
}

async fn resolve_model(deps: &Deps, cfg: &WorkerConfig) -> Option<ModelRef> {
    if let Some(m) = &cfg.default_model {
        // Validate against the live list; fall through to the picker if absent.
        if let Ok(models) = router::list_models(&deps.iii, cfg.timeout_ms).await {
            if models
                .iter()
                .any(|x| x.id == m.id && x.provider == m.provider)
            {
                return Some(m.clone());
            }
            tracing::warn!(model = %m.id, "default_model not offered by router; using first available");
        } else {
            return Some(m.clone());
        }
    }
    let models = router::list_models(&deps.iii, cfg.timeout_ms).await.ok()?;
    models.into_iter().next().map(|m| ModelRef {
        provider: m.provider,
        id: m.id,
    })
}

async fn fetch_thread(deps: &Deps, channel: &str, thread_ts: &str, max: u32) -> Vec<PendingMsg> {
    let resp: Value = match slack::call(
        deps,
        "conversations.replies",
        json!({ "channel": channel, "ts": thread_ts, "limit": max.max(1) }),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "conversations.replies backfill failed");
            return Vec::new();
        }
    };
    let Some(messages) = resp.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    messages
        .iter()
        .filter_map(|m| {
            let text = m.get("text").and_then(Value::as_str)?.trim();
            if text.is_empty() {
                return None;
            }
            Some(PendingMsg {
                user: m.get("user").and_then(Value::as_str).map(str::to_string),
                text: text.to_string(),
            })
        })
        .collect()
}

fn compose_message(context: &[PendingMsg], user_text: &str) -> String {
    if context.is_empty() {
        return user_text.to_string();
    }
    let mut out = String::from("Conversation so far in this Slack thread:\n");
    for m in context {
        let who = m.user.as_deref().unwrap_or("user");
        out.push_str(&format!("- {who}: {}\n", m.text));
    }
    out.push_str("\nNow the user addresses you:\n");
    out.push_str(user_text);
    out
}

fn build_system_prompt(
    cfg: &WorkerConfig,
    channel: &str,
    thread_ts: &str,
    session_id: Option<&str>,
) -> String {
    let ctx = CHANNEL_CONTEXT
        .replace("{channel}", channel)
        .replace("{thread_ts}", thread_ts)
        .replace("{session_id}", session_id.unwrap_or("(new)"));
    match &cfg.system_prompt {
        Some(extra) if !extra.trim().is_empty() => format!("{ctx}\n\n{extra}"),
        _ => ctx,
    }
}

/// Remove the bot's `<@BOTID>` mention(s) from the text and tidy whitespace left
/// behind (e.g. the gap in "hey <@U1> do x").
fn strip_mention(text: &str, bot_id: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(id) = bot_id {
        out = out.replace(&format!("<@{id}>"), "");
        // Collapse spaces/tabs left by the removed mention, but keep newlines.
        while out.contains("  ") {
            out = out.replace("  ", " ");
        }
        out = out.replace(" \n", "\n").replace("\n ", "\n");
    }
    out.trim().to_string()
}

fn truncate_front(v: &mut Vec<PendingMsg>, max: usize) {
    if v.len() > max {
        let drop = v.len() - max;
        v.drain(0..drop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_mention_removes_bot_tag() {
        assert_eq!(strip_mention("<@U1> hello", Some("U1")), "hello");
        assert_eq!(strip_mention("hi there", Some("U1")), "hi there");
    }

    #[test]
    fn strip_mention_collapses_interior_gap() {
        // Mention in the middle must not leave a double space.
        assert_eq!(strip_mention("hey <@U1> do x", Some("U1")), "hey do x");
        // Multiple mentions of the bot are all removed.
        assert_eq!(strip_mention("<@U1> a <@U1> b", Some("U1")), "a b");
    }

    #[test]
    fn strip_mention_preserves_newlines() {
        // Newlines (e.g. code blocks) survive; only the mention gap is tidied.
        assert_eq!(
            strip_mention("<@U1> line1\nline2", Some("U1")),
            "line1\nline2"
        );
    }

    #[test]
    fn strip_mention_keeps_other_user_mentions() {
        // Only the bot's own id is stripped; other mentions stay.
        assert_eq!(strip_mention("<@U1> ping <@U2>", Some("U1")), "ping <@U2>");
    }

    #[test]
    fn strip_mention_noop_without_bot_id() {
        assert_eq!(strip_mention("<@U1> hello", None), "<@U1> hello");
    }

    #[test]
    fn compose_message_includes_context() {
        let ctx = vec![PendingMsg {
            user: Some("U9".into()),
            text: "earlier note".into(),
        }];
        let msg = compose_message(&ctx, "do the thing");
        assert!(msg.contains("earlier note"));
        assert!(msg.contains("do the thing"));
    }

    #[test]
    fn compose_message_plain_when_no_context() {
        assert_eq!(compose_message(&[], "hello"), "hello");
    }
}
