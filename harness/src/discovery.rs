//! Reactive function-registry cache. Native exposure needs the set of callable
//! functions to build the model's tools; instead of re-listing the registry on
//! every turn, the harness keeps a cached snapshot and lets the engine push
//! changes. `engine::functions-available` fires when functions are
//! registered/unregistered, and the handler re-fetches the authoritative
//! `engine::functions::list` and swaps the snapshot. This mirrors the
//! `configuration` hot-reload pattern (a shared cell + a targeted refresh).
//!
//! Re-fetching `engine::functions::list` on each change — rather than trusting
//! the trigger's payload snapshot — keeps the cache identical to what the
//! per-turn list returned (same internal-hiding filter), so the loop's read is
//! an exact drop-in.
//!
//! The trigger only fires ON CHANGE, so the snapshot is seeded once at boot;
//! after that the trigger keeps it live. [`build_tools`](crate::turn_loop)
//! reads it under [`Deps::functions`](crate::deps::Deps::functions).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::json;
use tokio::sync::RwLock;

use crate::clients::{EngineClient, FunctionDescriptor};

/// Hot-swappable function-registry snapshot shared with the turn loop.
pub type FunctionsCell = Arc<RwLock<Arc<Vec<FunctionDescriptor>>>>;

const FUNCTIONS_FN_ID: &str = "harness::on-functions-change";
const FUNCTIONS_TRIGGER_TYPE: &str = "engine::functions-available";

/// An empty registry snapshot — seeded at boot, then kept live by the trigger.
pub fn new_cell() -> FunctionsCell {
    Arc::new(RwLock::new(Arc::new(Vec::new())))
}

/// Swap the snapshot under the write lock.
pub async fn apply(cell: &FunctionsCell, functions: Vec<FunctionDescriptor>) {
    *cell.write().await = Arc::new(functions);
}

/// Fetch the authoritative registry and swap the snapshot; returns the count.
async fn reload(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) -> usize {
    let engine = EngineClient::new(iii.clone(), timeout_ms);
    let functions = engine.functions_list().await;
    let count = functions.len();
    apply(cell, functions).await;
    count
}

/// Seed the snapshot from the registry. The trigger fires only on change, so
/// without this the cache would stay empty until the first change.
pub async fn seed(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) {
    let count = reload(iii, cell, timeout_ms).await;
    tracing::info!(count, "seeded function-registry cache");
}

/// Internal `harness::on-functions-change` payload. The handler re-fetches the
/// authoritative registry, so the (advisory) event tag is the only field.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnFunctionsChangeEvent {
    /// Engine event tag (advisory; the handler re-fetches the full list).
    #[serde(default)]
    pub event: Option<String>,
}

/// Ack returned by the internal `harness::on-functions-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnFunctionsChangeResponse {
    pub ok: bool,
}

/// Register the internal change handler and bind the
/// `engine::functions-available` trigger. Best-effort: a failed bind warns (the
/// seeded snapshot still serves — it just won't update until restart) rather
/// than bricking boot. The handler is tagged `internal` so it stays off the
/// public catalog (and out of the very cache it maintains).
pub fn register_functions_trigger(iii: &Arc<IIIClient>, cell: FunctionsCell, timeout_ms: u64) {
    let engine = iii.clone();
    iii.register_function(
        FUNCTIONS_FN_ID,
        RegisterFunction::new_async(move |_event: OnFunctionsChangeEvent| {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                let count = reload(&engine, &cell, timeout_ms).await;
                tracing::debug!(count, "function-registry cache refreshed");
                Ok::<OnFunctionsChangeResponse, Error>(OnFunctionsChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: refresh the cached function-registry snapshot when functions are \
             registered/unregistered (driven by the engine::functions-available trigger).",
        )
        .metadata(json!({ "internal": true })),
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: FUNCTIONS_TRIGGER_TYPE.to_string(),
        function_id: FUNCTIONS_FN_ID.to_string(),
        config: json!({}),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            trigger_type = FUNCTIONS_TRIGGER_TYPE,
            function_id = FUNCTIONS_FN_ID,
            "function-registry change trigger bound"
        ),
        Err(e) => tracing::warn!(
            trigger_type = FUNCTIONS_TRIGGER_TYPE,
            error = %e,
            "binding engine::functions-available failed; cache will not auto-refresh"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_swaps_snapshot() {
        let cell = new_cell();
        assert!(cell.read().await.is_empty());

        apply(
            &cell,
            vec![FunctionDescriptor {
                function_id: "shell::run".into(),
                description: Some("run a command".into()),
                parameters: None,
            }],
        )
        .await;

        let snap = cell.read().await.clone();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].function_id, "shell::run");
    }
}
