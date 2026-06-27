//! `harness::notify_agent` — the ONE shared subscription fire handler.
//!
//! Every subscription binds (via `engine::register_trigger`) to this single
//! function. The engine's per-subscription proxy injects the registration
//! `metadata` into the fired payload under [`TRIGGER_META_KEY`]
//! (`__metadata`), so this handler recovers which session/subscription a fire
//! belongs to from the payload — no per-subscription function is registered.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::subscriptions::{NOTIFY_AGENT_DESC, NOTIFY_AGENT_ID, TRIGGER_META_KEY};
use crate::types::message::AgentMessage;

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

/// Handle a fired subscription: recover its context from `__metadata`, build the
/// notification, inject it into the owning session, and self-tear-down (`once`).
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

    let sub_id = meta
        .get("subscription_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let session_id = meta
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (Some(sub_id), Some(session_id)) = (sub_id, session_id) else {
        tracing::warn!("notify_agent metadata missing subscription_id/session_id; dropping");
        return;
    };
    let label = meta
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string);
    let once = meta.get("once").and_then(Value::as_bool).unwrap_or(false);

    // Claim the fire: `once` removes the entry (winner-takes-all → at-most-once),
    // recurring just bumps the sequence (and confirms the entry is still live).
    let entry_id = if once {
        let Some((_session, trigger_id)) = deps.subscriptions.take(&sub_id) else {
            return; // already torn down; a concurrent fire won
        };
        if let Some(trigger_id) = trigger_id {
            if let Err(e) = deps
                .engine()
                .await
                .dispatch(
                    crate::functions::subscribe::UNREGISTER_TRIGGER_ID,
                    json!({ "id": trigger_id }),
                )
                .await
            {
                tracing::warn!(sub_id, error = %e, "once subscription teardown failed");
            }
        }
        format!("e_notify_{sub_id}")
    } else {
        let Some(seq) = deps.subscriptions.next_seq(&sub_id) else {
            return; // torn down between fire and handling
        };
        format!("e_notify_{sub_id}_{seq}")
    };

    let summary = summarize_event(&event);
    let text = match &label {
        Some(label) => format!("[notification: {label}] {summary}"),
        None => format!("[notification] {summary}"),
    };
    let message = AgentMessage::user_text(text);
    let origin = json!({
        "notification": { "label": label, "subscription_id": sub_id }
    });

    if let Err(e) =
        crate::functions::send::inject(deps, &session_id, message, Some(&entry_id), Some(&origin))
            .await
    {
        tracing::warn!(
            sub_id,
            session_id = %session_id,
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
}
