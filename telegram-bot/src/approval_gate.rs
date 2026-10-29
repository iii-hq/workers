//! Presence probe for the optional standalone `approval-gate` worker.
//!
//! The gate owns the `approval::*` functions; telegram-bot must run cleanly
//! with or without it. Mirrors the console's `useApprovalGateStatus` /
//! `isApprovalGateAvailable` pattern.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Deliver, IIIClient, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

/// Engine worker name for the standalone approval-gate worker.
pub const APPROVAL_GATE_WORKER_NAME: &str = "approval-gate";

const WATCH_FN_ID: &str = "telegram-bot::__on-approval-gate-worker";

/// The engine's worker lifecycle source.
const ON_WORKERS_CHANGED_FN: &str = "engine::on-workers-changed";

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
    /// safe to call. False during the initial presence probe and while the
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
/// to call. False during the initial presence probe and while the gate is
/// absent.
pub fn is_approval_gate_available(status: &ApprovalGateStatus) -> bool {
    status.present.load(Ordering::Relaxed) && !status.loading.load(Ordering::Relaxed)
}

/// One-time presence probe via `engine::workers::list`.
pub async fn probe_presence(iii: &IIIClient) -> bool {
    match list_workers(iii).await {
        Ok(workers) => has_approval_gate(&workers),
        Err(e) => {
            tracing::warn!(error = %e, "approval-gate presence probe failed");
            false
        }
    }
}

/// An `engine::on-workers-changed` event. It names the worker by id only, so
/// the handler re-reads `engine::workers::list` for names.
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct WorkersChangedEvent {
    #[serde(default)]
    #[allow(dead_code)]
    event: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    worker_id: Option<String>,
}

#[derive(Debug, serde::Serialize, JsonSchema)]
struct WorkerLifecycleAck {
    ok: bool,
}

/// The worker changes that can add or remove a worker name: a connection, its
/// metadata (which carries the name) and a disconnection.
fn lifecycle_filters() -> Value {
    json!({
        "input.event": {
            "$in": ["worker_connected", "worker_metadata_updated", "worker_disconnected"]
        }
    })
}

/// Observe `engine::on-workers-changed` so a connecting or disconnecting
/// `approval-gate` flips presence. The approval observers themselves are
/// created once at boot and stay `pending` while the gate is absent.
pub fn start_watch(iii: Arc<IIIClient>, status: Arc<ApprovalGateStatus>) {
    let engine = iii.clone();
    iii.register_function(
        WATCH_FN_ID,
        RegisterFunction::new_async(move |_event: WorkersChangedEvent| {
            let status = status.clone();
            let engine = engine.clone();
            async move {
                refresh_presence(&engine, &status).await;
                Ok::<_, Error>(WorkerLifecycleAck { ok: true })
            }
        })
        .description("Internal: track approval-gate worker connect/disconnect."),
    );

    // The observer lives as long as the worker's connection; dropping the
    // handle does not dispose it.
    iii.observe(
        Source::function(ON_WORKERS_CHANGED_FN)
            .filters(lifecycle_filters())
            .deliver(Deliver::Output),
        WATCH_FN_ID,
    );
}

/// Re-read the worker list and record whether the gate is in it. A failed
/// read keeps the last known presence.
async fn refresh_presence(iii: &IIIClient, status: &ApprovalGateStatus) {
    match list_workers(iii).await {
        Ok(workers) => {
            apply_presence(status, has_approval_gate(&workers));
        }
        Err(e) => {
            tracing::warn!(error = %e, "approval-gate presence refresh failed; keeping last state")
        }
    }
}

/// Record `present`; returns whether it changed.
fn apply_presence(status: &ApprovalGateStatus, present: bool) -> bool {
    let was = status.present.swap(present, Ordering::Relaxed);
    if was == present {
        return false;
    }
    if present {
        tracing::info!("approval-gate connected");
    } else {
        tracing::info!("approval-gate disconnected — approval prompts disabled");
    }
    true
}

fn has_approval_gate(workers: &[EngineWorker]) -> bool {
    workers
        .iter()
        .any(|w| w.name.as_deref() == Some(APPROVAL_GATE_WORKER_NAME))
}

#[derive(Debug, Deserialize)]
struct EngineWorker {
    #[serde(default)]
    name: Option<String>,
}

async fn list_workers(iii: &IIIClient) -> Result<Vec<EngineWorker>, Error> {
    let result = iii
        .call(CallRequest {
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

    fn worker(name: &str) -> EngineWorker {
        EngineWorker {
            name: Some(name.into()),
        }
    }

    #[test]
    fn is_available_requires_present_and_not_loading() {
        assert!(!is_approval_gate_available(&status_with(false, false)));
        assert!(!is_approval_gate_available(&status_with(true, true)));
        assert!(!is_approval_gate_available(&status_with(false, true)));
        assert!(is_approval_gate_available(&status_with(true, false)));
    }

    #[test]
    fn finds_the_approval_gate_by_name() {
        assert!(has_approval_gate(&[
            worker("harness"),
            worker("approval-gate")
        ]));
    }

    #[test]
    fn ignores_unrelated_workers() {
        assert!(!has_approval_gate(&[worker("harness")]));
        assert!(!has_approval_gate(&[EngineWorker { name: None }]));
    }

    #[test]
    fn presence_changes_are_reported_once() {
        let status = status_with(false, false);
        assert!(apply_presence(&status, true));
        assert!(status.is_available());
        assert!(!apply_presence(&status, true));
        assert!(apply_presence(&status, false));
        assert!(!status.is_available());
    }

    #[test]
    fn watch_selects_connections_metadata_and_disconnections() {
        assert_eq!(
            lifecycle_filters(),
            json!({
                "input.event": {
                    "$in": ["worker_connected", "worker_metadata_updated", "worker_disconnected"]
                }
            })
        );
    }
}
