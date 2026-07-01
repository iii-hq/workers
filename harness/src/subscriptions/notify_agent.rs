//! `harness::notify_agent` — the ONE shared subscription fire handler.
//!
//! Every subscription binds (via `engine::register_trigger`) to this single
//! function. The engine stores the registration `metadata` on the `Trigger`
//! and delivers it at fire time as a distinct invocation argument (not folded
//! into the fired payload). That trusted metadata carries the subscription id,
//! owning session, label, and one-shot semantics; the local registry validates
//! liveness and ownership for cleanup.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::subscriptions::{NOTIFY_AGENT_DESC, NOTIFY_AGENT_ID};
use crate::types::message::AgentMessage;

#[derive(Debug, Deserialize, Serialize)]
struct NotifyMetadata {
    subscription_id: String,
    session_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    once: bool,
}

pub fn register(deps: Arc<Deps>) {
    let iii = deps.iii.clone();
    iii.register_function(
        NOTIFY_AGENT_ID,
        RegisterFunction::new_async_with_metadata(move |event: Value, metadata: Option<Value>| {
            let deps = deps.clone();
            async move {
                on_fire(&deps, event, metadata).await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        })
        .description(NOTIFY_AGENT_DESC),
    );
}

async fn on_fire(deps: &Deps, event: Value, metadata: Option<Value>) {
    let meta = match parse_metadata(metadata) {
        Ok(meta) => meta,
        Err(MetadataError::Missing) => {
            tracing::warn!("notify_agent fire without trigger metadata; dropping");
            return;
        }
        Err(MetadataError::Invalid(e)) => {
            tracing::warn!(error = %e, "notify_agent metadata invalid; dropping");
            return;
        }
    };

    let Some(claim) =
        deps.subscriptions
            .claim_fire(&meta.subscription_id, &meta.session_id, meta.once)
    else {
        return;
    };
    if let Some(trigger_id) = claim.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }

    let (message, origin) = notification_message(&meta, &event);

    if let Err(e) = crate::functions::send::inject(
        deps,
        &meta.session_id,
        message,
        Some(&claim.entry_id),
        Some(&origin),
    )
    .await
    {
        tracing::warn!(
            sub_id = %meta.subscription_id,
            session_id = %meta.session_id,
            error = %e,
            "subscription notification injection failed"
        );
    }
}

#[derive(Debug)]
enum MetadataError {
    Missing,
    Invalid(serde_json::Error),
}

fn parse_metadata(metadata: Option<Value>) -> Result<NotifyMetadata, MetadataError> {
    let meta = metadata.ok_or(MetadataError::Missing)?;
    serde_json::from_value(meta).map_err(MetadataError::Invalid)
}

fn notification_message(meta: &NotifyMetadata, event: &Value) -> (AgentMessage, Value) {
    let summary = summarize_event(event);
    let text = match &meta.label {
        Some(label) => format!("[notification: {label}] {summary}"),
        None => format!("[notification] {summary}"),
    };
    (
        AgentMessage::user_text(text),
        json!({ "notification": true }),
    )
}

fn summarize_event(event: &Value) -> String {
    const MAX: usize = 600;
    let rendered = match event {
        Value::Null => "event fired".to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "event fired".to_string()),
    };
    if rendered.chars().count() > MAX {
        let mut s: String = rendered.chars().take(MAX).collect();
        s.push_str(" …(truncated)");
        s
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::content::ContentBlock;

    fn text_of(message: AgentMessage) -> String {
        let AgentMessage::User(user) = message else {
            panic!("expected user message");
        };
        ContentBlock::join_text(&user.content)
    }

    #[test]
    fn metadata_is_required_and_validated() {
        assert!(matches!(parse_metadata(None), Err(MetadataError::Missing)));

        let invalid = Some(json!({ "session_id": "s" }));
        assert!(matches!(
            parse_metadata(invalid),
            Err(MetadataError::Invalid(_))
        ));

        let valid = Some(json!({
            "subscription_id": "sub_1",
            "session_id": "s_secret",
            "label": "done"
        }));
        let meta = parse_metadata(valid).unwrap();

        assert_eq!(meta.subscription_id, "sub_1");
        assert_eq!(meta.session_id, "s_secret");
        assert_eq!(meta.label.as_deref(), Some("done"));
    }

    #[test]
    fn notification_message_uses_label_and_origin() {
        let meta = NotifyMetadata {
            subscription_id: "sub_1".to_string(),
            session_id: "s_1".to_string(),
            label: Some("job finished".to_string()),
            once: true,
        };
        let event = json!({ "event_type": "set", "value": { "done": true } });

        let (message, origin) = notification_message(&meta, &event);

        assert_eq!(origin, json!({ "notification": true }));
        assert_eq!(
            text_of(message),
            r#"[notification: job finished] {"event_type":"set","value":{"done":true}}"#
        );
    }
}
