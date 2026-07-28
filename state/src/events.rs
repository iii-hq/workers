//! State change-event fan-out. Port of the builtin's invoke_triggers
//! (state.rs:445-566): triggers_enabled gate first, scope/key pre-filter,
//! one spawned task per event, sequential bindings, condition semantics via
//! crate::condition (only explicit `false` blocks; errors skip + log).

use std::sync::Arc;

use iii_sdk::IIIClient;
use iii_sdk::protocol::TriggerRequest;
use serde_json::Value;

use crate::structs::StateEventData;
use crate::trigger::{TriggerTable, matches};

/// Abstracts `iii.trigger` so fan-out is unit-testable without an engine.
///
/// `metadata` is the binding's registration metadata, delivered as the
/// fire-time sidecar (engine `fire_triggers` parity: `call_with_metadata`).
/// Handlers like `harness::spawn` / `harness::notify_agent` resolve which
/// reaction or subscription fired from it and no-op without it, so a plain
/// call here silently kills every state-triggered reaction.
#[async_trait::async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Value, String>;
}

pub struct SdkInvoker {
    pub iii: Arc<IIIClient>,
}

#[async_trait::async_trait]
impl Invoker for SdkInvoker {
    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Value, String> {
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: None,
        };
        match metadata {
            Some(m) => self.iii.trigger(request.metadata(m)).await,
            None => self.iii.trigger(request).await,
        }
        .map_err(|e| e.to_string())
    }
}

pub async fn fan_out(
    invoker: Arc<dyn Invoker>,
    triggers: &TriggerTable,
    triggers_enabled: bool,
    event: StateEventData,
) {
    // LIVE gate (builtin parity): checked before any trigger collection.
    if !triggers_enabled {
        tracing::debug!("state trigger fan-out disabled via configuration; skipping");
        return;
    }

    // Collect matching bindings, releasing the lock before spawning.
    let matched: Vec<(String, Option<String>, Option<Value>)> = {
        let guard = triggers.read().await;
        guard
            .values()
            .filter(|t| matches(&t.config, &event.scope, &event.key))
            .map(|t| {
                (
                    t.function_id.clone(),
                    t.config.condition_function_id.clone(),
                    t.metadata.clone(),
                )
            })
            .collect()
    };
    if matched.is_empty() {
        return; // no span, no task — same short-circuit as the builtin
    }

    let Ok(event_json) = serde_json::to_value(&event) else {
        tracing::error!("Failed to convert state event data to value");
        return;
    };

    tokio::spawn(async move {
        for (function_id, condition_id, metadata) in matched {
            if let Some(cond) = &condition_id {
                // condition::check maps: null/no-result -> true, only `false`
                // blocks; Err -> skip this binding and log (builtin parity).
                // Conditions are plain function calls, not trigger fires — no
                // sidecar.
                match invoker.call(cond, event_json.clone(), None).await {
                    Ok(v) if !v.is_null() && v.as_bool() == Some(false) => continue,
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(condition_function_id = %cond, error = %e,
                            "Error invoking condition function");
                        continue;
                    }
                }
            }
            if let Err(e) = invoker
                .call(&function_id, event_json.clone(), metadata)
                .await
            {
                tracing::error!(function_id = %function_id, error = %e,
                    "Error invoking trigger handler");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::{StateEventData, StateEventType};
    use crate::trigger::{StateTriggerEntry, StateTriggerSpec};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct RecordingInvoker {
        calls: AtomicUsize,
        last: tokio::sync::Mutex<Option<(String, serde_json::Value, Option<serde_json::Value>)>>,
        condition_result: Option<serde_json::Value>,
    }

    #[async_trait::async_trait]
    impl Invoker for RecordingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: serde_json::Value,
            metadata: Option<serde_json::Value>,
        ) -> Result<serde_json::Value, String> {
            if function_id.starts_with("cond::") {
                return Ok(self
                    .condition_result
                    .clone()
                    .unwrap_or(serde_json::Value::Null));
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().await = Some((function_id.to_string(), payload, metadata));
            Ok(serde_json::Value::Null)
        }
    }

    fn event(scope: &str, key: &str) -> StateEventData {
        StateEventData {
            message_type: "state".to_string(),
            event_type: StateEventType::Created,
            scope: scope.to_string(),
            key: key.to_string(),
            old_value: None,
            new_value: serde_json::json!(1),
        }
    }

    fn table(entries: Vec<(&str, StateTriggerEntry)>) -> crate::trigger::TriggerTable {
        Arc::new(tokio::sync::RwLock::new(
            entries
                .into_iter()
                .map(|(id, e)| (id.to_string(), e))
                .collect::<HashMap<_, _>>(),
        ))
    }

    fn entry(scope: Option<&str>, key: Option<&str>, cond: Option<&str>) -> StateTriggerEntry {
        StateTriggerEntry {
            config: StateTriggerSpec {
                scope: scope.map(String::from),
                key: key.map(String::from),
                condition_function_id: cond.map(String::from),
            },
            function_id: "backend::on_change".to_string(),
            metadata: None,
        }
    }

    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }

    #[tokio::test]
    async fn fires_matching_binding_with_event_payload() {
        let inv = Arc::new(RecordingInvoker::default());
        let t = table(vec![("t1", entry(Some("orders"), None, None))]);
        fan_out(inv.clone(), &t, true, event("orders", "o1")).await;
        settle().await;
        assert_eq!(inv.calls.load(Ordering::SeqCst), 1);
        let (fid, payload, metadata) = inv.last.lock().await.clone().unwrap();
        assert_eq!(fid, "backend::on_change");
        assert_eq!(payload["type"], "state");
        assert_eq!(payload["event_type"], "state:created");
        assert_eq!(payload["scope"], "orders");
        assert_eq!(payload["key"], "o1");
        assert!(metadata.is_none(), "binding without metadata fires plain");
    }

    /// Registration metadata must reach the handler as the fire-time sidecar.
    /// `harness::spawn` / `harness::notify_agent` resolve the reaction or
    /// subscription from it and silently no-op without it — dropping it here
    /// killed every state-triggered reaction (rctest-x7k2 postmortem).
    #[tokio::test]
    async fn fires_with_registration_metadata_sidecar() {
        let inv = Arc::new(RecordingInvoker::default());
        let mut e = entry(Some("orders"), Some("change"), None);
        e.metadata = Some(serde_json::json!({"task": "spawn a reactor", "session_id": "r1"}));
        let t = table(vec![("t1", e)]);
        fan_out(inv.clone(), &t, true, event("orders", "change")).await;
        settle().await;
        assert_eq!(inv.calls.load(Ordering::SeqCst), 1);
        let (_, _, metadata) = inv.last.lock().await.clone().unwrap();
        let m = metadata.expect("registration metadata must be delivered at fire time");
        assert_eq!(m["task"], "spawn a reactor");
        assert_eq!(m["session_id"], "r1");
    }

    #[tokio::test]
    async fn scope_mismatch_and_disabled_gate_skip_fan_out() {
        let inv = Arc::new(RecordingInvoker::default());
        let t = table(vec![("t1", entry(Some("orders"), None, None))]);
        fan_out(inv.clone(), &t, true, event("users", "u1")).await; // no match
        fan_out(inv.clone(), &t, false, event("orders", "o1")).await; // gate off
        settle().await;
        assert_eq!(inv.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn condition_false_blocks_null_passes() {
        let blocked = Arc::new(RecordingInvoker {
            condition_result: Some(serde_json::json!(false)),
            ..Default::default()
        });
        let t = table(vec![("t1", entry(None, None, Some("cond::c")))]);
        fan_out(blocked.clone(), &t, true, event("s", "k")).await;
        settle().await;
        assert_eq!(
            blocked.calls.load(Ordering::SeqCst),
            0,
            "explicit false must block"
        );

        let passed = Arc::new(RecordingInvoker::default()); // condition returns Null
        fan_out(passed.clone(), &t, true, event("s", "k")).await;
        settle().await;
        assert_eq!(
            passed.calls.load(Ordering::SeqCst),
            1,
            "null/no-result must pass"
        );
    }
}
