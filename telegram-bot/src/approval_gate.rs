//! Presence probe for the optional standalone `approval-gate` worker.
//!
//! The gate owns the `approval::*` functions; telegram-bot must run cleanly
//! with or without it. Mirrors the console's `useApprovalGateStatus` /
//! `isApprovalGateAvailable` pattern.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

/// Engine worker name for the standalone approval-gate worker.
pub const APPROVAL_GATE_WORKER_NAME: &str = "approval-gate";

const WATCH_FN_ID: &str = "telegram-bot::__on-approval-gate-worker";

/// Whether the `approval-gate` worker is currently connected.
pub struct ApprovalGateStatus {
    present: AtomicBool,
    loading: AtomicBool,
    bound: AtomicBool,
}

impl ApprovalGateStatus {
    pub fn new() -> Self {
        Self {
            present: AtomicBool::new(false),
            loading: AtomicBool::new(true),
            bound: AtomicBool::new(false),
        }
    }

    pub fn set_present(&self, present: bool) {
        self.present.store(present, Ordering::Relaxed);
    }

    /// Sets presence and returns what it was.
    fn swap_present(&self, present: bool) -> bool {
        self.present.swap(present, Ordering::Relaxed)
    }

    /// True only the first time: the gate's triggers are bound once per
    /// process. The engine parks those bindings while the gate is away and
    /// replays them when it registers again, so binding again would deliver
    /// every event twice.
    pub fn take_first_bind(&self) -> bool {
        !self.bound.swap(true, Ordering::Relaxed)
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

/// Presence probe via `engine::workers::list`; a failed probe reads as absent.
pub async fn probe_presence(iii: &IIIClient) -> bool {
    gate_listed(iii).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "approval-gate presence probe failed");
        false
    })
}

async fn gate_listed(iii: &IIIClient) -> Result<bool, Error> {
    Ok(list_workers(iii)
        .await?
        .iter()
        .any(|w| w.name.as_deref() == Some(APPROVAL_GATE_WORKER_NAME)))
}

/// `engine::workers-available` payload. Every change re-probes, so nothing
/// in it is read; declared so the function publishes a typed schema.
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct WorkersChanged {
    #[serde(default)]
    #[allow(dead_code)]
    event: String,
}

#[derive(Debug, serde::Serialize, JsonSchema)]
struct WorkersChangedAck {
    ok: bool,
}

/// Bind the engine's `engine::workers-available` trigger type so presence
/// follows the gate: every worker connect, metadata update (when the gate's
/// name arrives) and disconnect re-probes `engine::workers::list`, and
/// `on_first_present` runs the first time the gate appears, unless the boot
/// probe already bound its triggers ([`ApprovalGateStatus::take_first_bind`]).
pub fn start_watch(
    iii: Arc<IIIClient>,
    status: Arc<ApprovalGateStatus>,
    on_first_present: Arc<dyn Fn() + Send + Sync>,
) {
    // One probe at a time, so the last one applied saw the latest state.
    let probing = Arc::new(tokio::sync::Mutex::new(()));
    let iii_probe = iii.clone();
    iii.register_function(
        WATCH_FN_ID,
        RegisterFunction::new_async(move |_change: WorkersChanged| {
            let (iii, status, on_present) =
                (iii_probe.clone(), status.clone(), on_first_present.clone());
            let probing = probing.clone();
            async move {
                // Off the trigger's path: the probe is a round trip.
                tokio::spawn(async move {
                    let _one = probing.lock().await;
                    match gate_listed(&iii).await {
                        Ok(present) => apply_presence(&status, &*on_present, present),
                        Err(e) => tracing::debug!(error = %e, "approval-gate re-probe failed"),
                    }
                });
                Ok::<_, Error>(WorkersChangedAck { ok: true })
            }
        })
        .description("Internal: track approval-gate presence on worker changes."),
    );

    tokio::spawn(async move {
        for attempt in 1..=5 {
            match iii.register_trigger(RegisterTriggerInput::new(
                "engine::workers-available".to_string(),
                WATCH_FN_ID.to_string(),
                json!({}),
            )) {
                Ok(_) => {
                    tracing::info!(
                        "subscribed to engine::workers-available for approval-gate presence"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        error = %e,
                        "failed to subscribe to engine::workers-available; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(attempt * 2)).await;
                }
            }
        }
        tracing::warn!(
            "exhausted retries subscribing to engine::workers-available; \
             approval-gate presence will not update at runtime"
        );
    });
}

fn apply_presence(status: &ApprovalGateStatus, on_present: &dyn Fn(), present: bool) {
    match (status.swap_present(present), present) {
        (false, true) => {
            tracing::info!("approval-gate connected");
            if status.take_first_bind() {
                on_present();
            }
        }
        (true, false) => {
            tracing::info!("approval-gate disconnected — approval prompts disabled");
        }
        _ => {}
    }
}

#[derive(Debug, Deserialize)]
struct EngineWorker {
    #[serde(default)]
    name: Option<String>,
}

async fn list_workers(iii: &IIIClient) -> Result<Vec<EngineWorker>, Error> {
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

    /// Presence follows each probe, and `on_present` (which binds the
    /// gate's triggers) runs on the first appearance only: the engine
    /// replays the binding when the gate comes back.
    #[test]
    fn the_gate_triggers_are_bound_on_its_first_appearance_only() {
        let status = status_with(false, false);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let on_present = || {
            calls.fetch_add(1, Ordering::Relaxed);
        };
        for (present, available, total_calls) in [
            (true, true, 1),
            (true, true, 1),
            (false, false, 1),
            (false, false, 1),
            (true, true, 1),
        ] {
            apply_presence(&status, &on_present, present);
            assert_eq!(status.is_available(), available);
            assert_eq!(calls.load(Ordering::Relaxed), total_calls);
        }

        // Bound at boot: a later reappearance binds nothing.
        let booted = status_with(true, false);
        assert!(booted.take_first_bind());
        apply_presence(&booted, &on_present, false);
        apply_presence(&booted, &on_present, true);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
