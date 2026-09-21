//! Trigger bindings, and keeping them.
//!
//! A binding can fail to register — the engine restarts, the trigger's owner
//! worker is not up yet — and a monitor that quietly stopped watching is
//! worse than one that never started. So every binding lives in a slot that
//! the recovery loop reconciles: bound when its source is enabled, dropped
//! when it is turned off, and retried on the next pass when it failed.

use std::sync::Arc;

use iii_config_client::BindingSlot;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::{functions, WorkerConfig};

/// The engine's own trigger types.
pub const TRACE_TRIGGER: &str = "trace";
pub const LOG_TRIGGER: &str = "log";
/// Owned by other workers, so their absence is a reason to retry rather than
/// to fail.
pub const TURN_COMPLETED_TRIGGER: &str = "harness::turn-completed";
pub const CRON_TRIGGER: &str = "cron";

#[derive(Clone, Default)]
pub struct Bindings {
    trace: BindingSlot,
    log: BindingSlot,
    turn_completed: BindingSlot,
    prune: BindingSlot,
}

impl Bindings {
    /// Reconcile every binding with the live configuration. Idempotent:
    /// called at boot, after each configuration change, and on every pass of
    /// the recovery loop.
    pub async fn reconcile(&self, iii: &Arc<IIIClient>, config: &WorkerConfig) {
        let enabled = config.enabled;

        self.trace.reconcile(
            enabled && config.sources.trace.enabled,
            || {
                iii_config_client::try_bind(
                    iii,
                    RegisterTriggerInput::new(
                        TRACE_TRIGGER.to_string(),
                        functions::ON_TRACE_ACTIVITY_ID.to_string(),
                        json!({ "status": "error" }),
                    ),
                )
            },
            "watching error spans",
            "stopped watching error spans",
        );

        self.log.reconcile(
            enabled && config.sources.log.enabled,
            || {
                iii_config_client::try_bind(
                    iii,
                    RegisterTriggerInput::new(
                        LOG_TRIGGER.to_string(),
                        functions::ON_LOG_ID.to_string(),
                        json!({ "level": "error" }),
                    ),
                )
            },
            "watching error logs",
            "stopped watching error logs",
        );

        self.turn_completed.reconcile(
            enabled,
            || {
                iii_config_client::try_bind(
                    iii,
                    RegisterTriggerInput::new(
                        TURN_COMPLETED_TRIGGER.to_string(),
                        functions::ON_TURN_COMPLETED_ID.to_string(),
                        json!({}),
                    ),
                )
            },
            "listening for finished investigation turns",
            "stopped listening for finished turns",
        );

        self.prune.reconcile(
            enabled,
            || {
                iii_config_client::try_bind(
                    iii,
                    RegisterTriggerInput::new(
                        CRON_TRIGGER.to_string(),
                        functions::ON_SCHEDULE_ID.to_string(),
                        json!({ "expression": config.retention.cron }),
                    ),
                )
            },
            "daily prune scheduled",
            "daily prune unscheduled",
        );
    }

    /// Which sources are currently bound, for the status surface.
    pub fn bound(&self) -> (bool, bool) {
        (self.trace.is_bound(), self.log.is_bound())
    }
}

/// Whether the worker that owns a trigger type is registered yet. A binding
/// to a type nobody owns fails, so the recovery loop uses this to keep the
/// retry quiet until it can succeed.
pub async fn trigger_type_available(iii: &IIIClient, trigger_type: &str) -> bool {
    let Ok(response) = iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: json!({ "include_internal": true }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    else {
        return false;
    };
    response
        .get("triggers")
        .and_then(Value::as_array)
        .map(|triggers| {
            triggers.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str) == Some(trigger_type)
                    || entry.get("trigger_type").and_then(Value::as_str) == Some(trigger_type)
            })
        })
        .unwrap_or(false)
}
