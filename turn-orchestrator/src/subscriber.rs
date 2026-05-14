//! Subscriber on `turn::step_requested`. One trigger event = one state
//! transition. After running the transition the subscriber re-publishes
//! the topic if the record is not terminal.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, Value, III};
use serde_json::json;

use crate::config::TurnOrchestratorConfig;
use crate::persistence;
use crate::run_start::publish_step;
use crate::transitions;

pub const FUNCTION_ID: &str = "turn::step";

pub async fn execute(
    iii: III,
    cfg: Arc<TurnOrchestratorConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
    let session_id = extract_session_id(&payload).ok_or_else(|| {
        IIIError::Handler("turn::step_requested payload missing session_id".into())
    })?;

    let mut record = match persistence::load_record(&iii, &session_id).await {
        Some(r) => r,
        None => {
            tracing::warn!(%session_id, "turn::step_requested for unknown session");
            return Ok(json!({ "ok": false, "reason": "unknown_session" }));
        }
    };

    if record.is_terminal() {
        return Ok(json!({ "ok": true, "terminal": true }));
    }

    let from_state = record.state;
    transitions::step(&iii, &cfg, &mut record)
        .await
        .map_err(|e| {
            IIIError::Handler(format!(
                "transition from {} failed: {e}",
                from_state.as_str()
            ))
        })?;
    persistence::save_record(&iii, &record).await;

    if !record.is_terminal() {
        publish_step(&iii, &session_id).await;
    }
    Ok(json!({
        "ok": true,
        "from_state": from_state.as_str(),
        "to_state": record.state.as_str(),
    }))
}

pub fn register(iii: &III, cfg: &Arc<TurnOrchestratorConfig>) {
    let iii_for_handler = iii.clone();
    let cfg_for_handler = Arc::clone(cfg);
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "Run one durable state machine transition for a session.".to_string(),
        ),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            let cfg = Arc::clone(&cfg_for_handler);
            async move { execute(iii, cfg, payload).await }
        },
    ));
}

fn extract_session_id(payload: &Value) -> Option<String> {
    // `iii::durable::publish` delivers the `data` object here. Accept optional extra nesting
    // (`payload` / `data`) for compatibility with older in-memory pub/sub shapes.
    let inner = payload
        .get("payload")
        .or_else(|| payload.get("data"))
        .unwrap_or(payload);
    inner
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_register_accepts_config_arc() {
        // Compile-time pin: register() must take an Arc<TurnOrchestratorConfig>.
        // This guards against silently dropping config plumbing.
        fn _assert_signature(
            iii: &iii_sdk::III,
            cfg: &std::sync::Arc<crate::config::TurnOrchestratorConfig>,
        ) {
            super::register(iii, cfg);
        }
    }

    #[test]
    fn extract_session_id_from_nested_payload() {
        let p = json!({ "payload": { "session_id": "s1" }});
        assert_eq!(extract_session_id(&p).as_deref(), Some("s1"));
    }

    #[test]
    fn extract_session_id_from_top_level() {
        let p = json!({ "session_id": "s2" });
        assert_eq!(extract_session_id(&p).as_deref(), Some("s2"));
    }

    #[test]
    fn extract_session_id_returns_none_when_missing() {
        assert!(extract_session_id(&json!({})).is_none());
    }
}
