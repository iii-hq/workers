//! Presence probe for the optional standalone `approval-gate` worker.
//!
//! The gate owns the `approval::*` functions; telegram-bot must run cleanly
//! with or without it. Mirrors the console's `useApprovalGateStatus` /
//! `isApprovalGateAvailable` pattern.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

/// Engine worker name for the standalone approval-gate worker.
pub const APPROVAL_GATE_WORKER_NAME: &str = "approval-gate";

const WATCH_FN_ID: &str = "telegram-bot::__on-approval-gate-worker";

/// Whether the `approval-gate` worker is currently connected.
pub struct ApprovalGateStatus {
    present: AtomicBool,
    loading: AtomicBool,
}

impl ApprovalGateStatus {
    pub fn new() -> Self {
        Self {
            present: AtomicBool::new(false),
            loading: AtomicBool::new(true),
        }
    }

    pub fn set_present(&self, present: bool) {
        self.present.store(present, Ordering::Relaxed);
    }

    pub fn finish_loading(&self) {
        self.loading.store(false, Ordering::Relaxed);
    }

    /// Whether the approval-gate's `approval::*` functions are registered and
    /// safe to trigger. False during the initial presence probe and while the
    /// gate is absent.
    pub fn is_available(&self) -> bool {
        is_approval_gate_available(self)
    }
}

impl Default for ApprovalGateStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the approval-gate's `approval::*` functions are registered and safe
/// to trigger. False during the initial presence probe and while the gate is
/// absent.
pub fn is_approval_gate_available(status: &ApprovalGateStatus) -> bool {
    status.present.load(Ordering::Relaxed) && !status.loading.load(Ordering::Relaxed)
}

/// One-time presence probe via `engine::workers::list`.
pub async fn probe_presence(iii: &III) -> bool {
    match list_workers(iii).await {
        Ok(workers) => workers
            .iter()
            .any(|w| w.name.as_deref() == Some(APPROVAL_GATE_WORKER_NAME)),
        Err(e) => {
            tracing::warn!(error = %e, "approval-gate presence probe failed");
            false
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct WorkerLifecycleEvent {
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    worker: Option<String>,
    #[serde(default)]
    source: Option<Value>,
}

#[derive(Debug, serde::Serialize, JsonSchema)]
struct WorkerLifecycleAck {
    ok: bool,
}

/// Subscribe to engine `worker` lifecycle events so add/remove of
/// `approval-gate` flips presence and invokes `on_present` when the gate
/// connects.
pub fn start_watch(
    iii: Arc<III>,
    status: Arc<ApprovalGateStatus>,
    on_present: Arc<dyn Fn() + Send + Sync>,
) {
    iii.register_function(
        WATCH_FN_ID,
        RegisterFunction::new_async(move |event: WorkerLifecycleEvent| {
            let status = status.clone();
            let on_present = on_present.clone();
            async move {
                handle_worker_event(&status, &on_present, &event);
                Ok::<_, IIIError>(WorkerLifecycleAck { ok: true })
            }
        })
        .description("Internal: track approval-gate worker add/remove."),
    );

    tokio::spawn(async move {
        for attempt in 1..=5 {
            match iii.register_trigger(RegisterTriggerInput {
                trigger_type: "worker".to_string(),
                function_id: WATCH_FN_ID.to_string(),
                config: json!({
                    "operations": ["add", "remove"],
                    "stages": ["done"]
                }),
                metadata: None,
            }) {
                Ok(_) => {
                    tracing::info!("subscribed to worker trigger for approval-gate presence");
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        error = %e,
                        "failed to subscribe to worker trigger for approval-gate; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(attempt * 2)).await;
                }
            }
        }
        tracing::warn!(
            "exhausted retries subscribing to worker trigger; \
             approval-gate presence will not update at runtime"
        );
    });
}

fn handle_worker_event(
    status: &ApprovalGateStatus,
    on_present: &Arc<dyn Fn() + Send + Sync>,
    event: &WorkerLifecycleEvent,
) {
    if !is_approval_gate_event(event) || event.stage.as_deref() != Some("done") {
        return;
    }
    match event.operation.as_deref() {
        Some("add") => {
            status.set_present(true);
            tracing::info!("approval-gate connected");
            on_present();
        }
        Some("remove") => {
            status.set_present(false);
            tracing::info!("approval-gate disconnected — approval prompts disabled");
        }
        _ => {}
    }
}

fn is_approval_gate_event(event: &WorkerLifecycleEvent) -> bool {
    if let Some(w) = event.worker.as_deref() {
        let lower = w.to_ascii_lowercase();
        if lower == APPROVAL_GATE_WORKER_NAME || lower.contains(APPROVAL_GATE_WORKER_NAME) {
            return true;
        }
    }
    if let Some(source) = &event.source {
        if let Ok(text) = serde_json::to_string(source) {
            if text
                .to_ascii_lowercase()
                .contains(APPROVAL_GATE_WORKER_NAME)
            {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Deserialize)]
struct EngineWorker {
    #[serde(default)]
    name: Option<String>,
}

async fn list_workers(iii: &III) -> Result<Vec<EngineWorker>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("workers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_with(present: bool, loading: bool) -> ApprovalGateStatus {
        let s = ApprovalGateStatus::new();
        s.set_present(present);
        if !loading {
            s.finish_loading();
        }
        s
    }

    #[test]
    fn is_available_requires_present_and_not_loading() {
        assert!(!is_approval_gate_available(&status_with(false, false)));
        assert!(!is_approval_gate_available(&status_with(true, true)));
        assert!(!is_approval_gate_available(&status_with(false, true)));
        assert!(is_approval_gate_available(&status_with(true, false)));
    }

    #[test]
    fn matches_approval_gate_worker_name() {
        let evt = WorkerLifecycleEvent {
            worker: Some("approval-gate".into()),
            ..Default::default()
        };
        assert!(is_approval_gate_event(&evt));
    }

    #[test]
    fn matches_approval_gate_in_worker_substring() {
        let evt = WorkerLifecycleEvent {
            worker: Some("path/approval-gate".into()),
            ..Default::default()
        };
        assert!(is_approval_gate_event(&evt));
    }

    #[test]
    fn ignores_unrelated_worker() {
        let evt = WorkerLifecycleEvent {
            worker: Some("harness".into()),
            ..Default::default()
        };
        assert!(!is_approval_gate_event(&evt));
    }

    #[test]
    fn matches_approval_gate_in_source_json() {
        let evt = WorkerLifecycleEvent {
            source: Some(json!({ "name": "approval-gate" })),
            ..Default::default()
        };
        assert!(is_approval_gate_event(&evt));
    }
}
