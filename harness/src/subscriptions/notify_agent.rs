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
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::subscriptions::{NOTIFY_AGENT_DESC, NOTIFY_AGENT_ID};
use crate::surface::schema_value;
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

/// Fired-event payload of a subscription: arbitrary JSON produced by the
/// originating trigger (a state change, cron fire, log line, …), summarized
/// verbatim into the notification injected into the owning session.
///
/// The handler keeps a lossless `serde_json::Value` signature — a typed
/// struct would reject legitimate non-object payloads — so this newtype
/// exists only to document the wire schema (`schemars` would otherwise emit
/// the AnyValue schema, which the registry publish gate rejects).
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct NotifyAgentEvent(pub Value);

impl JsonSchema for NotifyAgentEvent {
    fn schema_name() -> String {
        "NotifyAgentEvent".to_string()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(
                vec![
                    InstanceType::Null,
                    InstanceType::Boolean,
                    InstanceType::Number,
                    InstanceType::String,
                    InstanceType::Array,
                    InstanceType::Object,
                ]
                .into(),
            ),
            metadata: Some(Box::new(Metadata {
                description: Some(
                    "Arbitrary fired-event payload from the subscribed trigger; delivered \
                     verbatim to the owning agent as a notification."
                        .to_string(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// Ack returned by `harness::notify_agent`. Fires are ack'd even when dropped
/// (stale, foreign, or malformed subscriptions log and return ok).
#[derive(Debug, Serialize, JsonSchema)]
pub struct NotifyAgentResponse {
    pub ok: bool,
}

pub fn register(deps: Arc<Deps>) {
    let iii = deps.iii.clone();
    iii.register_function(
        NOTIFY_AGENT_ID,
        RegisterFunction::new_async(move |event: Value, metadata: Option<Value>| {
            let deps = deps.clone();
            async move {
                on_fire(&deps, event, metadata).await;
                Ok::<Value, Error>(json!(NotifyAgentResponse { ok: true }))
            }
        })
        .description(NOTIFY_AGENT_DESC)
        .request_format(schema_value::<NotifyAgentEvent>())
        .response_format(schema_value::<NotifyAgentResponse>()),
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

    match crate::functions::send::inject(
        deps,
        &meta.session_id,
        message,
        Some(&claim.entry_id),
        Some(&origin),
    )
    .await
    {
        Err(e) => {
            tracing::warn!(
                sub_id = %meta.subscription_id,
                session_id = %meta.session_id,
                error = %e,
                "subscription notification injection failed"
            );
        }
        Ok(_) => {
            // ponytail: no fire-rate gate here (react has FireGate) — a hot
            // subscription pays one get_turn + two comm appends per fire; add
            // a gate if a real source ever runs hot.
            let cfg = deps.cfg().await;
            let root = crate::state::get_turn(&deps.iii, &meta.session_id, cfg.session_timeout_ms)
                .await
                .ok()
                .flatten()
                .map(|r| r.root().to_string())
                .unwrap_or_else(|| meta.session_id.clone());
            let trigger = crate::comm::CommTrigger {
                registered_trigger_id: claim
                    .trigger_id
                    .clone()
                    .or(Some(meta.subscription_id.clone())),
                trigger_type: None,
                label: meta.label.clone(),
                action: "notify".to_string(),
                child_session_id: None,
            };
            let to = crate::comm::CommEndpoint {
                session_id: meta.session_id.clone(),
                turn_id: None,
            };
            deps.comm
                .emit(
                    crate::comm::CommEvent {
                        seq: 0,
                        at: crate::comm::now_ms(),
                        root_session_id: root.clone(),
                        kind: crate::comm::CommKind::TriggerFire,
                        from: None,
                        to: Some(to.clone()),
                        trigger: Some(trigger.clone()),
                        summary: Some(crate::comm::snippet(&event)),
                        r#ref: None,
                    },
                    cfg.session_timeout_ms,
                )
                .await;
            deps.comm
                .emit(
                    crate::comm::CommEvent {
                        seq: 0,
                        at: crate::comm::now_ms(),
                        root_session_id: root,
                        kind: crate::comm::CommKind::Notify,
                        from: None,
                        to: Some(to),
                        trigger: Some(trigger),
                        summary: meta
                            .label
                            .clone()
                            .or_else(|| Some(crate::comm::snippet(&event))),
                        r#ref: None,
                    },
                    cfg.session_timeout_ms,
                )
                .await;
        }
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

    /// Mirrors the registry publish gate (`collect_worker_interface.py`):
    /// a schema is "typed" only if it carries a schema-defining keyword.
    /// `harness::notify_agent` is not in `surface::catalog()` (it is
    /// subscription plumbing, not agent-facing), so guard it here.
    #[test]
    fn wire_schemas_pass_the_publish_typed_gate() {
        const SCHEMA_DEFINING_KEYS: &[&str] = &[
            "type",
            "properties",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "enum",
            "items",
            "const",
        ];
        for (field, schema) in [
            ("request_schema", schema_value::<NotifyAgentEvent>()),
            ("response_schema", schema_value::<NotifyAgentResponse>()),
        ] {
            let obj = schema
                .as_object()
                .unwrap_or_else(|| panic!("harness::notify_agent {field} must be a JSON object"));
            assert!(
                SCHEMA_DEFINING_KEYS.iter().any(|k| obj.contains_key(*k)),
                "harness::notify_agent {field} is untyped (AnyValue/empty): {schema}"
            );
        }
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
