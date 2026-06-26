//! The per-subscription handler factory and the notification injection.
//!
//! Each subscription registers its OWN `harness::sub::<id>` function. Built-in
//! triggers fire the bound function with only the engine event payload (no
//! metadata redelivery), so the engine routing IS the addressing: the closure
//! captures the owning session and already knows whom to notify.

use std::sync::Arc;

use iii_sdk::{FunctionRef, IIIError, RegisterFunction};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::types::message::AgentMessage;

/// Register the unique internal handler `harness::sub::<sub_id>`.
pub fn register_subscription_handler(deps: Arc<Deps>, sub_id: String, fn_id: &str) -> FunctionRef {
    let iii = deps.iii.clone();
    iii.register_function(
        fn_id,
        RegisterFunction::new_async(move |event: Value| {
            let deps = deps.clone();
            let sub_id = sub_id.clone();
            async move {
                on_fire(&deps, &sub_id, event).await;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .description(
            "Internal: ephemeral subscription event handler — injects a notification into the \
             owning session when its trigger fires. Not called directly.",
        ),
    )
}

/// Handle a fired subscription: coalesce / expire, build the notification,
/// inject it into the owning session, and self-tear-down when `once`.
async fn on_fire(deps: &Deps, sub_id: &str, event: Value) {
    let registry = &deps.subscriptions;

    let Some(entry) = registry.get(sub_id) else {
        // Already torn down; the engine may still hold a stale binding briefly.
        return;
    };
    let Some((fire_count, once)) = registry.record_fire(sub_id) else {
        return;
    };

    let summary = summarize_event(&event);
    let text = match &entry.label {
        Some(label) => format!("[notification: {label}] {summary}"),
        None => format!("[notification] {summary}"),
    };
    let message = AgentMessage::user_text(text);
    // Deterministic entry id so a redelivered identical fire is idempotent.
    let entry_id = format!("e_notify_{sub_id}_{fire_count}");

    if let Err(e) =
        crate::functions::send::inject(deps, &entry.session_id, message, Some(&entry_id)).await
    {
        tracing::warn!(
            sub_id,
            session_id = %entry.session_id,
            error = %e,
            "subscription notification injection failed"
        );
    }

    if once {
        registry.remove(sub_id);
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
}
