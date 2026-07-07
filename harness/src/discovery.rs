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
//! an exact drop-in. `list` carries no parameter schemas, so each descriptor is
//! then hydrated from `engine::functions::info` for native exposure.
//!
//! The snapshot also carries a `generation` counter: [`apply`] bumps it only
//! when the function set's content fingerprint changes, so the turn loop can
//! notice a mid-conversation registry change and tell the model its cached
//! contracts may be stale.
//!
//! The trigger only fires ON CHANGE, so the snapshot is seeded once at boot;
//! after that the trigger keeps it live (plus a low-frequency safety reload).
//! [`build_tools`](crate::turn_loop) reads it under
//! [`Deps::functions`](crate::deps::Deps::functions).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::clients::{EngineClient, FunctionDescriptor};

/// A function-registry snapshot: the callable set plus a monotonic generation
/// that only advances when the set's content changes.
pub struct FunctionsSnapshot {
    pub functions: Vec<FunctionDescriptor>,
    pub generation: u64,
    /// Content fingerprint gating the generation bump (private to `apply`).
    fingerprint: u64,
}

/// Hot-swappable function-registry snapshot shared with the turn loop.
pub type FunctionsCell = Arc<RwLock<Arc<FunctionsSnapshot>>>;

const FUNCTIONS_FN_ID: &str = "harness::on-functions-change";
const FUNCTIONS_TRIGGER_TYPE: &str = "engine::functions-available";
const SAFETY_RELOAD_SECS: u64 = 300;

impl FunctionsSnapshot {
    /// A snapshot literal for unit tests (the fingerprint field is private).
    #[cfg(test)]
    pub fn for_tests(functions: Vec<FunctionDescriptor>) -> Self {
        Self {
            fingerprint: fingerprint_of(&functions),
            functions,
            generation: 1,
        }
    }
}

/// An empty registry snapshot (generation 0) — seeded at boot, then kept live
/// by the trigger. The boot seed bumps it to 1 on the first non-empty apply.
pub fn new_cell() -> FunctionsCell {
    Arc::new(RwLock::new(Arc::new(FunctionsSnapshot {
        functions: Vec::new(),
        generation: 0,
        fingerprint: fingerprint_of(&[]),
    })))
}

/// Content fingerprint over the sorted (id, description, serialized schema)
/// tuples — stable across reloads of an unchanged registry, so a re-apply of
/// the same set is a no-op.
fn fingerprint_of(functions: &[FunctionDescriptor]) -> u64 {
    let mut tuples: Vec<(&str, &str, String)> = functions
        .iter()
        .map(|f| {
            (
                f.function_id.as_str(),
                f.description.as_deref().unwrap_or(""),
                f.parameters
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default(),
            )
        })
        .collect();
    tuples.sort();
    let mut hasher = DefaultHasher::new();
    tuples.hash(&mut hasher);
    hasher.finish()
}

/// Swap the snapshot under the write lock, bumping the generation only when the
/// incoming set's fingerprint differs from the current one.
pub async fn apply(cell: &FunctionsCell, functions: Vec<FunctionDescriptor>) {
    let fingerprint = fingerprint_of(&functions);
    let mut guard = cell.write().await;
    if fingerprint == guard.fingerprint {
        return;
    }
    let generation = guard.generation + 1;
    *guard = Arc::new(FunctionsSnapshot {
        functions,
        generation,
        fingerprint,
    });
}

/// The already-hydrated `id -> parameters` map from the current snapshot,
/// used to carry schemas forward across a reload without re-fetching.
async fn prev_params(cell: &FunctionsCell) -> HashMap<String, Value> {
    cell.read()
        .await
        .functions
        .iter()
        .filter_map(|d| d.parameters.clone().map(|p| (d.function_id.clone(), p)))
        .collect()
}

/// Carry a prior `parameters` forward for any id still `None`; report the ids
/// that need an `engine::functions::info` fan-out. Every descriptor is retained
/// in the output — a still-`None` one is flagged for fetch, never dropped.
fn plan_hydration(
    functions: Vec<FunctionDescriptor>,
    prev: &HashMap<String, Value>,
) -> (Vec<FunctionDescriptor>, Vec<String>) {
    let mut needs_fetch = Vec::new();
    let carried = functions
        .into_iter()
        .map(|mut d| {
            if d.parameters.is_none() {
                match prev.get(&d.function_id) {
                    Some(p) => d.parameters = Some(p.clone()),
                    None => needs_fetch.push(d.function_id.clone()),
                }
            }
            d
        })
        .collect();
    (carried, needs_fetch)
}

/// The engine's `function_ids` batch cap (`engine::functions::info`).
const INFO_BATCH_MAX: usize = 32;

/// Fill each descriptor's `parameters`, carrying already-hydrated schemas
/// forward and fetching only new/unresolved ids — one `engine::functions::info`
/// `function_ids` batch per 32 ids, with a per-id fallback for engines that
/// predate batch support. A per-id failure keeps the descriptor with
/// `parameters: None`.
// ponytail: engine include_schemas-on-list would replace this with the one list call reload already makes
async fn hydrate(
    engine: &EngineClient,
    cell: &FunctionsCell,
    functions: Vec<FunctionDescriptor>,
) -> Vec<FunctionDescriptor> {
    let prev = prev_params(cell).await;
    let (mut carried, needs_fetch) = plan_hydration(functions, &prev);
    if needs_fetch.is_empty() {
        return carried;
    }
    let mut fetched: HashMap<String, Value> = HashMap::new();
    let mut batch_supported = true;
    for chunk in needs_fetch.chunks(INFO_BATCH_MAX) {
        if batch_supported {
            match engine.functions_info_batch(chunk).await {
                Some(descriptors) => {
                    for d in descriptors {
                        if let Some(p) = d.parameters {
                            fetched.insert(d.function_id, p);
                        }
                    }
                    continue;
                }
                // Old engine (single-id only): fall through to per-id calls
                // for this and every later chunk.
                None => batch_supported = false,
            }
        }
        for id in chunk {
            if let Some(params) = engine.functions_info(id).await.and_then(|d| d.parameters) {
                fetched.insert(id.clone(), params);
            }
            // failure or schema-less: leave it None below (never dropped).
        }
    }
    for d in carried.iter_mut().filter(|d| d.parameters.is_none()) {
        if let Some(p) = fetched.get(&d.function_id) {
            d.parameters = Some(p.clone());
        }
    }
    carried
}

/// Fetch the authoritative registry, hydrate schemas, and swap the snapshot;
/// returns the count.
async fn reload(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) -> usize {
    let engine = EngineClient::new(iii.clone(), timeout_ms);
    let functions = engine.functions_list().await;
    let functions = hydrate(&engine, cell, functions).await;
    let count = functions.len();
    apply(cell, functions).await;
    count
}

/// Seed the snapshot from the registry. The trigger fires only on change, so
/// without this the cache would stay empty until the first change. The
/// unhydrated list is applied first for a fast boot (`build_tools` tolerates
/// `None` schemas), then hydrated and re-applied.
pub async fn seed(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) {
    let engine = EngineClient::new(iii.clone(), timeout_ms);
    let functions = engine.functions_list().await;
    apply(cell, functions.clone()).await;
    let hydrated = hydrate(&engine, cell, functions).await;
    let count = hydrated.len();
    apply(cell, hydrated).await;
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
    // ponytail: 5-min safety reload; the real fix is schema-aware functions_hash in the engine
    let reload_iii = iii.clone();
    let reload_cell = cell.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(SAFETY_RELOAD_SECS));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            let count = reload(&reload_iii, &reload_cell, timeout_ms).await;
            tracing::debug!(count, "function-registry cache safety-reloaded");
        }
    });

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

    fn desc(id: &str, parameters: Option<Value>) -> FunctionDescriptor {
        FunctionDescriptor {
            function_id: id.into(),
            description: None,
            parameters,
        }
    }

    #[tokio::test]
    async fn apply_swaps_snapshot() {
        let cell = new_cell();
        assert!(cell.read().await.functions.is_empty());

        apply(
            &cell,
            vec![desc("shell::run", Some(json!({ "type": "object" })))],
        )
        .await;

        let snap = cell.read().await.clone();
        assert_eq!(snap.functions.len(), 1);
        assert_eq!(snap.functions[0].function_id, "shell::run");
    }

    #[tokio::test]
    async fn apply_bumps_generation_only_on_change() {
        let cell = new_cell();
        assert_eq!(cell.read().await.generation, 0);

        let fns = vec![desc("a::b", Some(json!({ "type": "object" })))];
        apply(&cell, fns.clone()).await;
        assert_eq!(cell.read().await.generation, 1);

        // Identical re-apply: fingerprint matches, no bump.
        apply(&cell, fns).await;
        assert_eq!(cell.read().await.generation, 1);

        // Changed set: bump.
        apply(
            &cell,
            vec![
                desc("a::b", Some(json!({ "type": "object" }))),
                desc("c::d", None),
            ],
        )
        .await;
        assert_eq!(cell.read().await.generation, 2);
    }

    #[test]
    fn plan_hydration_carries_forward_and_retains_unresolved() {
        let prev: HashMap<String, Value> = [("a::b".to_string(), json!({ "type": "object" }))]
            .into_iter()
            .collect();
        let (carried, needs_fetch) =
            plan_hydration(vec![desc("a::b", None), desc("c::d", None)], &prev);

        assert_eq!(carried.len(), 2);
        let a = carried.iter().find(|d| d.function_id == "a::b").unwrap();
        assert_eq!(a.parameters, Some(json!({ "type": "object" })));
        // Not in prev: flagged for fetch AND retained with None (a failed
        // functions_info leaves it exactly here — the descriptor is never dropped).
        let c = carried.iter().find(|d| d.function_id == "c::d").unwrap();
        assert_eq!(c.parameters, None);
        assert_eq!(needs_fetch, vec!["c::d".to_string()]);
    }
}
