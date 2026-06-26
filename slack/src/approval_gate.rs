//! Presence of the optional standalone `approval-gate` worker. The bridge posts
//! inline Block Kit approvals only when the gate is connected; otherwise tool
//! calls are not held and no approval prompt is shown.

use std::sync::atomic::{AtomicBool, Ordering};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::json;

pub const APPROVAL_GATE_WORKER_NAME: &str = "approval-gate";

#[derive(Default)]
pub struct ApprovalGateStatus {
    present: AtomicBool,
}

impl ApprovalGateStatus {
    pub fn new() -> Self {
        Self {
            present: AtomicBool::new(false),
        }
    }

    pub fn set_present(&self, present: bool) {
        self.present.store(present, Ordering::Relaxed);
    }

    pub fn is_available(&self) -> bool {
        self.present.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
struct EngineWorker {
    #[serde(default)]
    name: Option<String>,
}

/// One-time presence probe via `engine::workers::list`.
pub async fn probe_presence(iii: &IIIClient) -> bool {
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
