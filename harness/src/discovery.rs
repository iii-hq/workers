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
//! A second `include_internal: true` list feeds [`FunctionsSnapshot::internal_ids`]:
//! presence only, no schemas, outside the fingerprint. An authorized internal
//! function the public inventory hides (`engine::functions::info`) then
//! resolves as present instead of removed, while the per-console ephemeral
//! internals (`console::*-watch::r<n>::…`) never become tools or bump the
//! generation. Presence never grants permission.
//!
//! The snapshot also carries a `generation` counter: [`apply`] bumps it only
//! when the function set's content fingerprint changes, so the turn loop can
//! notice a mid-conversation registry change and tell the model its cached
//! contracts may be stale.
//!
//! The trigger only fires ON CHANGE and has no catch-up snapshot, so the
//! snapshot is seeded once at boot; after that two event sources keep it
//! live, and no timer reloads anything:
//!
//! - `engine::functions-available` runs the internal handler. What fires it
//!   depends on the engine. Released engines up to v0.24.5-rc.2 poll every
//!   5 s and hash only the sorted function-ID set: they fire on adds and
//!   removes, but never when an existing id is re-registered with a new
//!   schema or description. Engines with registry-driven notification
//!   (iii-hq/iii#2283) fire on every registration, overwrites included, and
//!   on every removal, folding a burst into one event ~100 ms later. The
//!   payload carries no schemas either way, so the handler re-reads the
//!   registry and picks its hydration from what moved (`change_hydration`).
//! - Every worker announce (`engine::workers-available` →
//!   [`crate::engine_events`]) runs [`refresh`], which re-reads every schema
//!   rather than carrying the cached ones forward. On an id-set-only engine
//!   this is the only path that sees a restarted worker's new contracts under
//!   unchanged ids, so it stays even though newer engines report them too.
//!
//! Still invisible: on an id-set-only engine, a live worker re-registering an
//! existing id with a new schema without reconnecting (no event at all); on a
//! newer engine, such an overwrite folded into the same burst as an id add or
//! remove (the event reads as an id change), until the next announce or
//! overwrite of that id.
//!
//! [`build_tools`](crate::turn_loop) reads the snapshot under
//! [`Deps::functions`](crate::deps::Deps::functions).

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

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
    /// Every id the registry knows, internal ones included — presence only.
    /// Not part of the fingerprint: ephemeral internals must not bump it.
    pub internal_ids: BTreeSet<String>,
}

impl FunctionsSnapshot {
    /// One digest per function `policy` permits, sorted: what a session can
    /// call, each contract on its own. Comparing two of these tells a
    /// function that changed or left (a contract the session may hold went
    /// stale) from one that only joined (nothing the session holds changed).
    pub fn permitted_digests(&self, policy: &crate::policy::CompiledPolicy) -> Vec<u32> {
        let mut digests: Vec<u32> = self
            .functions
            .iter()
            .filter(|f| policy.allows(&f.function_id))
            .map(|f| fingerprint_of(std::iter::once(f)) as u32)
            .collect();
        digests.sort_unstable();
        digests.dedup();
        digests
    }
}

/// Hot-swappable function-registry snapshot shared with the turn loop.
pub type FunctionsCell = Arc<RwLock<Arc<FunctionsSnapshot>>>;

const FUNCTIONS_FN_ID: &str = "harness::on-functions-change";
const FUNCTIONS_TRIGGER_TYPE: &str = "engine::functions-available";

/// An empty registry snapshot (generation 0) — seeded at boot, then kept live
/// by the trigger. The boot seed bumps it to 1 on the first non-empty apply.
pub fn new_cell() -> FunctionsCell {
    Arc::new(RwLock::new(Arc::new(snapshot_of(Vec::new()))))
}

/// A standalone generation-0 snapshot over `functions`, with no internal ids.
pub fn snapshot_of(functions: Vec<FunctionDescriptor>) -> FunctionsSnapshot {
    FunctionsSnapshot {
        fingerprint: fingerprint_of(&functions),
        functions,
        generation: 0,
        internal_ids: BTreeSet::new(),
    }
}

/// Content fingerprint over the sorted (id, description, serialized schema)
/// tuples — stable across reloads of an unchanged registry, so a re-apply of
/// the same set is a no-op.
fn fingerprint_of<'a>(functions: impl IntoIterator<Item = &'a FunctionDescriptor>) -> u64 {
    let mut tuples: Vec<(&str, &str, String)> = functions
        .into_iter()
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
/// incoming set's fingerprint differs from the current one. Availability events
/// with a byte-identical set deliberately do NOT bump: the generation feeds the
/// registry-changed notice and the discovery hint, and a no-op bump invalidates
/// the provider's prompt-cache prefix for nothing. A response-schema-only
/// change is fingerprint-invisible (the cache keeps no response schemas), so
/// it goes un-noticed until a cached field moves.
pub async fn apply(cell: &FunctionsCell, functions: Vec<FunctionDescriptor>) {
    apply_with_internal(cell, functions, None).await;
}

/// [`apply`] plus the registry's full id set (`include_internal: true`) in ONE
/// write, so both halves of a snapshot always describe the same registry.
/// `None` — or an empty set, which a live registry never returns because it
/// also lists its public ids — means the internal list is unknown this round:
/// the previous set is kept minus every id the public list just dropped, so a
/// stale superset never hides a real removal. The internal set is presence
/// only, for [`crate::agents::effective_contract`], and never moves the
/// generation: per-console ephemeral internals churn constantly.
pub async fn apply_with_internal(
    cell: &FunctionsCell,
    functions: Vec<FunctionDescriptor>,
    internal_ids: Option<BTreeSet<String>>,
) {
    let fingerprint = fingerprint_of(&functions);
    let mut guard = cell.write().await;
    let internal_ids = match internal_ids.filter(|ids| !ids.is_empty()) {
        Some(ids) => ids,
        None => {
            let listed: HashSet<&str> = functions.iter().map(|d| d.function_id.as_str()).collect();
            let dropped: HashSet<&str> = guard
                .functions
                .iter()
                .map(|d| d.function_id.as_str())
                .filter(|id| !listed.contains(id))
                .collect();
            guard
                .internal_ids
                .iter()
                .filter(|id| !dropped.contains(id.as_str()))
                .cloned()
                .collect()
        }
    };
    if fingerprint == guard.fingerprint && internal_ids == guard.internal_ids {
        return;
    }
    let generation = guard.generation + u64::from(fingerprint != guard.fingerprint);
    *guard = Arc::new(FunctionsSnapshot {
        functions,
        generation,
        fingerprint,
        internal_ids,
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

/// Which schemas a reload re-reads from `engine::functions::info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hydration {
    /// Only ids with no cached schema: the id set changed, and the ids that
    /// stayed keep their cached schemas.
    Missing,
    /// Every id: a worker announced, or an existing id was re-registered,
    /// either of which may carry new schemas. Cached schemas stay as the
    /// fallback for a failed fetch, so a flaky read never reads as a
    /// contract change.
    All,
}

/// The hydration for a `functions-available` event, from what the fresh
/// registry read moved relative to the cached snapshot. The payload carries
/// no schemas, so it cannot say which contract changed. When neither the
/// public nor the full id set moved, the event can only be an overwrite of an
/// existing id (engines with registry-driven notification fire on those), so
/// every schema is re-read. Anything else is explained by an add or remove
/// and fetches only the missing schemas, which keeps the frequent internal
/// churn (per-console watches) cheap. An unknown full id set (a failed read)
/// keeps the cheap path. On an engine that hashes only the id set, an
/// unchanged set means an announce refresh already applied the change: the
/// re-read is redundant there, never wrong.
fn change_hydration(
    cached: &FunctionsSnapshot,
    functions: &[FunctionDescriptor],
    internal_ids: Option<&BTreeSet<String>>,
) -> Hydration {
    let Some(ids) = internal_ids.filter(|ids| !ids.is_empty()) else {
        return Hydration::Missing;
    };
    let public = |fs: &[FunctionDescriptor]| -> BTreeSet<String> {
        fs.iter().map(|d| d.function_id.clone()).collect()
    };
    if *ids == cached.internal_ids && public(functions) == public(&cached.functions) {
        Hydration::All
    } else {
        Hydration::Missing
    }
}

/// Carry a prior `parameters` forward for any id still `None`; report the ids
/// that need an `engine::functions::info` fan-out. Every descriptor is retained
/// in the output — a still-`None` one is flagged for fetch, never dropped.
fn plan_hydration(
    functions: Vec<FunctionDescriptor>,
    prev: &HashMap<String, Value>,
    mode: Hydration,
) -> (Vec<FunctionDescriptor>, Vec<String>) {
    let mut needs_fetch = Vec::new();
    let carried = functions
        .into_iter()
        .map(|mut d| {
            if d.parameters.is_none() {
                match prev.get(&d.function_id) {
                    Some(p) => {
                        d.parameters = Some(p.clone());
                        if mode == Hydration::All {
                            needs_fetch.push(d.function_id.clone());
                        }
                    }
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
    mode: Hydration,
) -> Vec<FunctionDescriptor> {
    let prev = prev_params(cell).await;
    let (mut carried, needs_fetch) = plan_hydration(functions, &prev, mode);
    if needs_fetch.is_empty() {
        return carried;
    }
    // A fetched schema-less descriptor is an answer too (its schema went
    // away); only a failed read keeps what the descriptor already carries.
    let mut fetched: HashMap<String, Option<Value>> = HashMap::new();
    let mut batch_supported = true;
    for chunk in needs_fetch.chunks(INFO_BATCH_MAX) {
        if batch_supported {
            match engine.functions_info_batch(chunk).await {
                Some(descriptors) => {
                    for d in descriptors {
                        fetched.insert(d.function_id, d.parameters);
                    }
                    continue;
                }
                // Old engine (single-id only): fall through to per-id calls
                // for this and every later chunk.
                None => batch_supported = false,
            }
        }
        for id in chunk {
            if let Some(d) = engine.functions_info(id).await {
                fetched.insert(id.clone(), d.parameters);
            }
            // failure: keep the carried value below (never dropped).
        }
    }
    apply_fetched(&mut carried, &fetched);
    carried
}

/// Overlay what `engine::functions::info` answered onto the carried
/// descriptors; an id it did not answer for keeps its carried schema.
fn apply_fetched(carried: &mut [FunctionDescriptor], fetched: &HashMap<String, Option<Value>>) {
    for d in carried.iter_mut() {
        if let Some(p) = fetched.get(&d.function_id) {
            d.parameters = p.clone();
        }
    }
}

/// Fetch the authoritative registry, hydrate schemas, and swap the snapshot;
/// returns the count. `mode: None` is a `functions-available` event: the
/// hydration is picked by [`change_hydration`] from what the read moved.
async fn reload(
    iii: &Arc<IIIClient>,
    cell: &FunctionsCell,
    timeout_ms: u64,
    mode: Option<Hydration>,
) -> usize {
    let engine = EngineClient::new(iii.clone(), timeout_ms);
    let (functions, internal_ids) =
        tokio::join!(engine.functions_list(), engine.internal_function_ids());
    let mode = match mode {
        Some(mode) => mode,
        None => {
            let cached = cell.read().await.clone();
            change_hydration(&cached, &functions, internal_ids.as_ref())
        }
    };
    let functions = hydrate(&engine, cell, functions, mode).await;
    let count = functions.len();
    apply_with_internal(cell, functions, internal_ids).await;
    count
}

/// Re-read the registry AND every schema — run on each worker announce,
/// where a restarted worker may have changed contracts under unchanged ids
/// (an engine that hashes only the id set never reports that).
/// The generation moves only if something actually changed.
pub async fn refresh(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) -> usize {
    reload(iii, cell, timeout_ms, Some(Hydration::All)).await
}

/// Seed the snapshot from the registry. The trigger fires only on change, so
/// without this the cache would stay empty until the first change. The
/// unhydrated list is applied first for a fast boot (`build_tools` tolerates
/// `None` schemas), then hydrated and re-applied.
pub async fn seed(iii: &Arc<IIIClient>, cell: &FunctionsCell, timeout_ms: u64) {
    let engine = EngineClient::new(iii.clone(), timeout_ms);
    let (functions, internal_ids) =
        tokio::join!(engine.functions_list(), engine.internal_function_ids());
    apply_with_internal(cell, functions.clone(), internal_ids).await;
    let hydrated = hydrate(&engine, cell, functions, Hydration::Missing).await;
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
/// seeded snapshot still serves, refreshed on worker announces) rather than
/// bricking boot. The handler is tagged `internal` so it stays off the public
/// catalog (and out of the very cache it maintains).
pub fn register_functions_trigger(iii: &Arc<IIIClient>, cell: FunctionsCell, timeout_ms: u64) {
    let engine = iii.clone();
    iii.register_function(
        FUNCTIONS_FN_ID,
        RegisterFunction::new_async(move |_event: OnFunctionsChangeEvent| {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                let count = reload(&engine, &cell, timeout_ms, None).await;
                tracing::debug!(count, "function-registry cache refreshed");
                Ok::<OnFunctionsChangeResponse, Error>(OnFunctionsChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: refresh the cached function-registry snapshot when functions are \
             registered, re-registered or unregistered (driven by the \
             engine::functions-available trigger).",
        )
        .metadata(json!({ "internal": true })),
    );

    match iii.register_trigger(RegisterTriggerInput::new(
        FUNCTIONS_TRIGGER_TYPE.to_string(),
        FUNCTIONS_FN_ID.to_string(),
        json!({}),
    )) {
        Ok(_) => tracing::info!(
            trigger_type = FUNCTIONS_TRIGGER_TYPE,
            function_id = FUNCTIONS_FN_ID,
            "function-registry change trigger bound"
        ),
        Err(e) => tracing::warn!(
            trigger_type = FUNCTIONS_TRIGGER_TYPE,
            error = %e,
            "binding engine::functions-available failed; cache refreshes only on worker announces"
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
    async fn internal_ids_are_presence_only_and_never_bump_generation() {
        let cell = new_cell();
        let fns = vec![desc("a::b", Some(json!({ "type": "object" })))];
        let ids = |extra: &[&str]| -> BTreeSet<String> {
            ["a::b", "engine::functions::info"]
                .iter()
                .chain(extra)
                .map(|id| id.to_string())
                .collect()
        };
        apply_with_internal(&cell, fns.clone(), Some(ids(&[]))).await;
        let snap = cell.read().await.clone();
        assert_eq!(snap.generation, 1);
        assert_eq!(snap.internal_ids, ids(&[]));
        // The same answer again is a no-op: no new snapshot, nothing to invalidate.
        apply_with_internal(&cell, fns.clone(), Some(ids(&[]))).await;
        assert!(Arc::ptr_eq(&snap, &*cell.read().await));
        // Internal churn alone (a console watch) never moves the generation.
        let churned = ids(&["console::watch::r1"]);
        apply_with_internal(&cell, fns.clone(), Some(churned.clone())).await;
        assert_eq!(cell.read().await.generation, 1);
        assert_eq!(cell.read().await.internal_ids, churned);
        // A failed or empty internal list is unknown, not "everything gone".
        apply_with_internal(&cell, fns.clone(), Some(BTreeSet::new())).await;
        apply(&cell, fns).await;
        assert_eq!(cell.read().await.internal_ids, churned);
        // ...but the kept set loses what the public list just dropped, so a
        // stale superset never masks a real removal.
        apply(&cell, vec![]).await;
        let snap = cell.read().await.clone();
        assert_eq!(snap.generation, 2);
        assert!(!snap.internal_ids.contains("a::b"));
        assert!(snap.internal_ids.contains("engine::functions::info"));
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

    #[tokio::test]
    async fn re_applying_an_identical_set_does_not_bump_generation() {
        // Availability events now route through this same `apply` (the forced
        // bump was removed), so this covers that path too. A no-op bump would
        // fire the registry-changed notice and re-arm the discovery hint,
        // invalidating the provider prompt-cache for nothing.
        let cell = new_cell();
        let fns = vec![desc("a::b", Some(json!({ "type": "object" })))];
        apply(&cell, fns.clone()).await;

        apply(&cell, fns).await;

        assert_eq!(cell.read().await.generation, 1);
    }

    /// A worker announce re-reads every schema: a restarted worker may have
    /// changed a contract under an unchanged id, which an engine that hashes
    /// only the id set never reports. The cached schema stays as the fallback.
    #[test]
    fn an_announce_refetches_every_schema_and_keeps_the_cache_as_fallback() {
        let prev: HashMap<String, Value> = [("a::b".to_string(), json!({ "type": "object" }))]
            .into_iter()
            .collect();
        let (mut carried, needs_fetch) = plan_hydration(
            vec![desc("a::b", None), desc("c::d", None)],
            &prev,
            Hydration::All,
        );
        assert_eq!(needs_fetch, vec!["a::b".to_string(), "c::d".to_string()]);
        // Only c::d answered: a::b keeps its cached schema, never `None`.
        let fetched: HashMap<String, Option<Value>> =
            [("c::d".to_string(), Some(json!({ "type": "string" })))]
                .into_iter()
                .collect();
        apply_fetched(&mut carried, &fetched);
        assert_eq!(carried[0].parameters, Some(json!({ "type": "object" })));
        assert_eq!(carried[1].parameters, Some(json!({ "type": "string" })));
        // An answered changed schema replaces the cached one.
        let changed: HashMap<String, Option<Value>> =
            [("a::b".to_string(), Some(json!({ "type": "array" })))]
                .into_iter()
                .collect();
        apply_fetched(&mut carried, &changed);
        assert_eq!(carried[0].parameters, Some(json!({ "type": "array" })));
    }

    #[test]
    fn plan_hydration_carries_forward_and_retains_unresolved() {
        let prev: HashMap<String, Value> = [("a::b".to_string(), json!({ "type": "object" }))]
            .into_iter()
            .collect();
        let (carried, needs_fetch) = plan_hydration(
            vec![desc("a::b", None), desc("c::d", None)],
            &prev,
            Hydration::Missing,
        );

        assert_eq!(carried.len(), 2);
        let a = carried.iter().find(|d| d.function_id == "a::b").unwrap();
        assert_eq!(a.parameters, Some(json!({ "type": "object" })));
        // Not in prev: flagged for fetch AND retained with None (a failed
        // functions_info leaves it exactly here — the descriptor is never dropped).
        let c = carried.iter().find(|d| d.function_id == "c::d").unwrap();
        assert_eq!(c.parameters, None);
        assert_eq!(needs_fetch, vec!["c::d".to_string()]);
    }

    #[test]
    fn an_event_that_moved_no_id_rereads_every_schema() {
        let ids =
            |list: &[&str]| -> BTreeSet<String> { list.iter().map(|id| id.to_string()).collect() };
        let mut cached = snapshot_of(vec![desc("a::b", Some(json!({ "type": "object" })))]);
        cached.internal_ids = ids(&["a::b", "engine::functions::info"]);
        let listed = vec![desc("a::b", None)];
        // Neither id set moved: only an overwrite of an existing id fires that.
        let same = cached.internal_ids.clone();
        assert_eq!(
            change_hydration(&cached, &listed, Some(&same)),
            Hydration::All
        );
        // Internal churn (a console watch) is an id change: cheap path.
        let churned = ids(&["a::b", "engine::functions::info", "console::watch::r1"]);
        assert_eq!(
            change_hydration(&cached, &listed, Some(&churned)),
            Hydration::Missing
        );
        // A public add is an id change too.
        let added = vec![desc("a::b", None), desc("c::d", None)];
        assert_eq!(
            change_hydration(&cached, &added, Some(&same)),
            Hydration::Missing
        );
        // An unknown full id set keeps the cheap path.
        assert_eq!(change_hydration(&cached, &listed, None), Hydration::Missing);
        assert_eq!(
            change_hydration(&cached, &listed, Some(&BTreeSet::new())),
            Hydration::Missing
        );
    }
}
