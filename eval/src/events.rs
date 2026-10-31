//! `eval::on-completed`: a session-monitor analysis reached a terminal status.
//!
//! A private function whose handler returns the event it is called with: only
//! this worker calls it, anyone may observe it, and observers select one
//! analysis with a filter on `input.evaluation_id`. The worker calls it only
//! while something observes it.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract::EvalStatusV1;

pub const ON_COMPLETED_FN: &str = "eval::on-completed";
pub const ON_COMPLETED_DESC: &str = "Called when a session-monitor analysis reaches a terminal \
     status (completed, failed or cancelled). No args; filter on input.evaluation_id or \
     input.status. Completion describes the monitor, not a validated improvement.";

/// The `eval::on-completed` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompletedEventV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
    /// Unix milliseconds when the analysis ended.
    pub timestamp: i64,
}

#[derive(Clone)]
pub struct EvalEvents {
    iii: Arc<IIIClient>,
}

impl EvalEvents {
    /// Registers `eval::on-completed`: private, no args, its handler returning
    /// the event it is called with.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        iii.register_function(
            ON_COMPLETED_FN,
            RegisterFunction::new(|event: CompletedEventV1| Ok::<_, Error>(event))
                .description(ON_COMPLETED_DESC)
                .private(),
        );
        Self { iii: iii.clone() }
    }

    /// Events that reach no observer, for tools that share the engine with a
    /// running `eval` and must not take over its function: nothing is
    /// registered, so nothing is observed and nothing is called.
    pub fn detached(iii: &Arc<IIIClient>) -> Self {
        Self { iii: iii.clone() }
    }

    pub async fn emit_completed(&self, evaluation_id: &str, status: EvalStatusV1) {
        if !self.iii.is_observed(ON_COMPLETED_FN) {
            return;
        }
        let event = CompletedEventV1 {
            evaluation_id: evaluation_id.to_owned(),
            status,
            timestamp: crate::ids::now_ms(),
        };
        let Ok(payload) = serde_json::to_value(&event) else {
            return;
        };
        let request = CallRequest {
            function_id: ON_COMPLETED_FN.to_owned(),
            payload,
            // Fire and forget: delivery does not wait for observers.
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::warn!(%evaluation_id, %error, "eval::on-completed call failed");
        }
    }
}
