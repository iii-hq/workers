//! `harness::notify_agent` — the ONE shared subscription fire handler.
//!
//! Every subscription binds (via `engine::register_trigger`) to this single
//! function. The engine's per-subscription proxy injects the registration
//! `metadata` into the fired payload under [`TRIGGER_META_KEY`]
//! (`__metadata`). That trusted metadata carries the subscription id, owning
//! session, label, and one-shot semantics; the local registry validates liveness
//! and ownership for cleanup.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::subscriptions::{NOTIFY_AGENT_DESC, NOTIFY_AGENT_ID, TRIGGER_META_KEY};
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

/// Register the single shared `harness::notify_agent` handler (once, at boot).
pub fn register(deps: Arc<Deps>) {
    let iii = deps.iii.clone();
    iii.register_function(
        NOTIFY_AGENT_ID,
        RegisterFunction::new_async(move |event: Value| {
            let deps = deps.clone();
            async move {
                on_fire(&deps, event).await;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .description(NOTIFY_AGENT_DESC),
    );
}

/// Handle a fired subscription: recover trusted context from `__metadata`,
/// validate it against the local registry, build the notification, inject it,
/// and self-tear-down (`once`).
async fn on_fire(deps: &Deps, mut event: Value) {
    // Extract and STRIP the engine-injected metadata so it never leaks into the
    // notification text shown to the agent.
    let meta = event
        .as_object_mut()
        .and_then(|o| o.remove(TRIGGER_META_KEY));
    let Some(meta) = meta else {
        tracing::warn!("notify_agent fire without {TRIGGER_META_KEY}; dropping");
        return;
    };

    let meta = match serde_json::from_value::<NotifyMetadata>(meta) {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(error = %e, "notify_agent metadata invalid; dropping");
            return;
        }
    };

    let Some(claim) =
        deps.subscriptions
            .claim_fire(&meta.subscription_id, &meta.session_id, meta.once)
    else {
        // Torn down, owner mismatch, or a concurrent one-shot fire won.
        return;
    };
    if let Some(trigger_id) = claim.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }

    let summary = summarize_event(&event);
    let text = match &meta.label {
        Some(label) => format!("[notification: {label}] {summary}"),
        None => format!("[notification] {summary}"),
    };
    let message = AgentMessage::user_text(text);
    let origin = json!({ "notification": true });

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

/// A compact one-line summary of the fired event for the notification text. The
/// full payload can be large or carry foreign data, so the excerpt is bounded.
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

    #[test]
    fn summarize_truncates_long_payloads() {
        let long = json!({ "blob": "x".repeat(5000) });
        let s = summarize_event(&long);
        assert!(s.ends_with("…(truncated)"));
        assert!(s.chars().count() <= 600 + " …(truncated)".chars().count());
    }

    #[test]
    fn summarize_passes_short_strings_through() {
        assert_eq!(summarize_event(&json!("done")), "done");
        assert_eq!(summarize_event(&Value::Null), "event fired");
    }

    #[test]
    fn summarize_excludes_stripped_metadata() {
        // `__metadata` is removed before summarizing, so addressing never leaks.
        let mut event = json!({ "event_type": "set", "__metadata": { "session_id": "s_secret" } });
        event.as_object_mut().unwrap().remove(TRIGGER_META_KEY);
        let s = summarize_event(&event);
        assert!(!s.contains("s_secret"));
        assert!(s.contains("event_type"));
    }

    #[test]
    fn notify_metadata_defaults_optional_fields() {
        let meta: NotifyMetadata =
            serde_json::from_value(json!({ "subscription_id": "sub_1", "session_id": "s_1" }))
                .unwrap();
        assert_eq!(meta.subscription_id, "sub_1");
        assert_eq!(meta.session_id, "s_1");
        assert_eq!(meta.label, None);
        assert!(!meta.once);
    }
}
