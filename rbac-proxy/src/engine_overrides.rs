//! Engine function overrides — the addition over the engine's RBAC contract.
//!
//! An engine RBAC listener gates *invocation* but only partially filters
//! *discovery*: a gated worker that can reach the discovery functions can still
//! enumerate functions, workers, and triggers it can never call. `rbac-proxy`
//! closes that gap by **result-filtering the eight discovery functions** to the
//! caller's boundaries, using the **same** vendored [`rbac::access_allowed`]
//! predicate as the invocation path — so the discovery surface and the
//! invocation surface can never disagree.
//!
//! Two of the eight cannot be filtered from their own response alone (a trigger
//! *type* carries no function binding; `engine::workers::list` carries only a
//! `function_count`, no ids), so this module also maintains two small TTL
//! caches over the proxy's **control connection**: a function catalog
//! (`function_id → { worker_name, metadata }`) and a binding index
//! (`trigger_type → [function_id]`).
//!
//! Results are rewritten as [`serde_json::Value`] so unknown fields survive —
//! the proxy only removes/edits the keys it must.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::config::WorkerConfig;
use crate::rbac::{self, ProxySession};

/// The eight discovery functions whose results are rewritten. The ninth
/// `EngineFunctions` id (`engine::workers::register`) is a carve-out write,
/// not a discovery surface, and is never rewritten.
pub const DISCOVERY_FUNCTIONS: &[&str] = &[
    "engine::functions::list",
    "engine::functions::info",
    "engine::workers::list",
    "engine::workers::info",
    "engine::triggers::list",
    "engine::triggers::info",
    "engine::registered-triggers::list",
    "engine::registered-triggers::info",
];

pub fn is_discovery(function_id: &str) -> bool {
    DISCOVERY_FUNCTIONS.contains(&function_id)
}

/// The outcome of filtering a discovery result: a rewritten result value, or a
/// replacement error (the caller may not see the queried entity).
#[derive(Debug, Clone, PartialEq)]
pub enum OverrideOutcome {
    Result(Value),
    Error { code: String, message: String },
}

impl OverrideOutcome {
    fn forbidden(function_id: &str) -> Self {
        // Engine parity: `engine::functions::info` returns FORBIDDEN for a
        // function the session may not see, NOT_FOUND only for one that does
        // not exist. The default matches a worker-gateway listener (a
        // deployment may opt into collapsing denied → NOT_FOUND to also hide
        // existence — an intentional divergence not enabled here).
        OverrideOutcome::Error {
            code: "FORBIDDEN".to_string(),
            message: format!(
                "function '{}' not allowed (add to rbac.expose_functions)",
                function_id
            ),
        }
    }

    fn not_found(message: &str) -> Self {
        OverrideOutcome::Error {
            code: "NOT_FOUND".to_string(),
            message: message.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Catalog & binding caches
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FnEntry {
    worker_name: String,
    metadata: Option<Value>,
}

#[derive(Default)]
struct FnState {
    entries: HashMap<String, FnEntry>,
    fetched_at: Option<Instant>,
}

#[derive(Default)]
struct BindState {
    /// `trigger_type → [target function_id]`.
    by_type: HashMap<String, Vec<String>>,
    fetched_at: Option<Instant>,
}

/// Short TTL — the discovery surface tolerates seconds of staleness, and a
/// metadata-gated function is briefly invisible (fail closed) until refresh.
const CACHE_TTL: Duration = Duration::from_secs(5);

/// TTL caches over the control connection. Keyed by the **engine** id
/// (prefixed where applicable, since that is what discovery results carry).
pub struct CatalogCache {
    iii: Arc<IIIClient>,
    functions: RwLock<FnState>,
    bindings: RwLock<BindState>,
    /// Serializes refreshes so a cold cache does not stampede the engine.
    refresh: Mutex<()>,
}

impl CatalogCache {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            functions: RwLock::new(FnState::default()),
            bindings: RwLock::new(BindState::default()),
            refresh: Mutex::new(()),
        }
    }

    /// Force a refresh on the next access (called on
    /// `engine::functions-available`).
    pub async fn invalidate(&self) {
        self.functions.write().await.fetched_at = None;
        self.bindings.write().await.fetched_at = None;
    }

    fn fresh(at: Option<Instant>) -> bool {
        at.is_some_and(|t| t.elapsed() < CACHE_TTL)
    }

    async fn ensure_functions(&self) {
        if Self::fresh(self.functions.read().await.fetched_at) {
            return;
        }
        let _guard = self.refresh.lock().await;
        if Self::fresh(self.functions.read().await.fetched_at) {
            return; // refreshed while we waited
        }
        match self.fetch_functions().await {
            Ok(entries) => {
                let mut st = self.functions.write().await;
                st.entries = entries;
                st.fetched_at = Some(Instant::now());
            }
            Err(e) => {
                tracing::warn!(error = %e, "catalog refresh (engine::functions::list) failed; serving stale/empty catalog")
            }
        }
    }

    async fn ensure_bindings(&self) {
        if Self::fresh(self.bindings.read().await.fetched_at) {
            return;
        }
        let _guard = self.refresh.lock().await;
        if Self::fresh(self.bindings.read().await.fetched_at) {
            return;
        }
        match self.fetch_bindings().await {
            Ok(by_type) => {
                let mut st = self.bindings.write().await;
                st.by_type = by_type;
                st.fetched_at = Some(Instant::now());
            }
            Err(e) => {
                tracing::warn!(error = %e, "binding refresh (engine::registered-triggers::list) failed; serving stale/empty index")
            }
        }
    }

    async fn fetch_functions(&self) -> Result<HashMap<String, FnEntry>, String> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        if let Some(arr) = resp.get("functions").and_then(Value::as_array) {
            for f in arr {
                let Some(id) = f.get("function_id").and_then(Value::as_str) else {
                    continue;
                };
                out.insert(
                    id.to_string(),
                    FnEntry {
                        worker_name: f
                            .get("worker_name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        metadata: f.get("metadata").cloned().filter(|m| !m.is_null()),
                    },
                );
            }
        }
        Ok(out)
    }

    async fn fetch_bindings(&self) -> Result<HashMap<String, Vec<String>>, String> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::registered-triggers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        let mut by_type: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(arr) = resp.get("registered_triggers").and_then(Value::as_array) {
            for t in arr {
                let (Some(tt), Some(fid)) = (
                    t.get("trigger_type").and_then(Value::as_str),
                    t.get("function_id").and_then(Value::as_str),
                ) else {
                    continue;
                };
                by_type
                    .entry(tt.to_string())
                    .or_default()
                    .push(fid.to_string());
            }
        }
        Ok(by_type)
    }

    /// The function's registered metadata (None on a cold cache → fail closed
    /// for metadata filters, open for wildcard filters which ignore metadata).
    pub async fn metadata_for(&self, function_id: &str) -> Option<Value> {
        self.ensure_functions().await;
        self.functions
            .read()
            .await
            .entries
            .get(function_id)
            .and_then(|e| e.metadata.clone())
    }

    /// All `(function_id, metadata)` owned by `worker_name`.
    async fn functions_of_worker(&self, worker_name: &str) -> Vec<(String, Option<Value>)> {
        self.ensure_functions().await;
        self.functions
            .read()
            .await
            .entries
            .iter()
            .filter(|(_, e)| e.worker_name == worker_name)
            .map(|(id, e)| (id.clone(), e.metadata.clone()))
            .collect()
    }

    /// All target function ids bound to `trigger_type`.
    async fn functions_for_trigger_type(&self, trigger_type: &str) -> Vec<String> {
        self.ensure_bindings().await;
        self.bindings
            .read()
            .await
            .by_type
            .get(trigger_type)
            .cloned()
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub async fn seed_functions(&self, entries: &[(&str, &str, Option<Value>)]) {
        let mut st = self.functions.write().await;
        st.entries = entries
            .iter()
            .map(|(id, wn, md)| {
                (
                    id.to_string(),
                    FnEntry {
                        worker_name: wn.to_string(),
                        metadata: md.clone(),
                    },
                )
            })
            .collect();
        st.fetched_at = Some(Instant::now());
    }

    #[cfg(test)]
    pub async fn seed_bindings(&self, by_type: &[(&str, &[&str])]) {
        let mut st = self.bindings.write().await;
        st.by_type = by_type
            .iter()
            .map(|(tt, fids)| (tt.to_string(), fids.iter().map(|s| s.to_string()).collect()))
            .collect();
        st.fetched_at = Some(Instant::now());
    }
}

// ---------------------------------------------------------------------------
// The per-function rewrite table
// ---------------------------------------------------------------------------

/// Filter a discovery result to the caller's boundaries. `function_id` is the
/// discovery function being answered; `result` is the engine's full,
/// unfiltered result.
pub async fn filter_result(
    function_id: &str,
    result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    match function_id {
        "engine::functions::list" => functions_list(result, session, cfg),
        "engine::functions::info" => functions_info(result, session, cfg),
        "engine::workers::list" => workers_list(result, session, cfg, catalog).await,
        "engine::workers::info" => workers_info(result, session, cfg, catalog).await,
        "engine::triggers::list" => OverrideOutcome::Result(result), // capability metadata; pass through
        "engine::triggers::info" => triggers_info(result, session, cfg, catalog).await,
        "engine::registered-triggers::list" => {
            registered_triggers_list(result, session, cfg, catalog).await
        }
        "engine::registered-triggers::info" => {
            registered_triggers_info(result, session, cfg, catalog).await
        }
        _ => OverrideOutcome::Result(result),
    }
}

/// `A(id, metadata)` against the live rbac config + this session.
fn allowed(cfg: &WorkerConfig, session: &ProxySession, id: &str, metadata: Option<&Value>) -> bool {
    rbac::access_allowed(Some(&cfg.rbac), session, id, metadata)
}

/// `A(id)` for an id whose metadata is not in the result (registered-triggers,
/// trigger-type bindings). Consults the catalog **only** when a metadata filter
/// is configured — wildcard-only configs need no round trip and a cold cache
/// must not fail them closed.
async fn allowed_via_cache(
    cfg: &WorkerConfig,
    session: &ProxySession,
    id: &str,
    catalog: &CatalogCache,
) -> bool {
    let md = if cfg.rbac.uses_metadata() {
        catalog.metadata_for(id).await
    } else {
        None
    };
    allowed(cfg, session, id, md.as_ref())
}

/// Strip the session's own `{prefix}::` from an id (a foreign id is unchanged).
fn strip(session: &ProxySession, id: &str) -> String {
    match session.function_registration_prefix.as_deref() {
        Some(p) => {
            let needle = format!("{p}::");
            id.strip_prefix(&needle).unwrap_or(id).to_string()
        }
        None => id.to_string(),
    }
}

fn strip_str_field(obj: &mut serde_json::Map<String, Value>, key: &str, session: &ProxySession) {
    if let Some(s) = obj.get(key).and_then(Value::as_str) {
        let stripped = strip(session, s);
        obj.insert(key.to_string(), Value::String(stripped));
    }
}

fn functions_list(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
) -> OverrideOutcome {
    let kept: Vec<Value> = result
        .get("functions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|f| {
                    let id = f.get("function_id").and_then(Value::as_str).unwrap_or("");
                    allowed(cfg, session, id, f.get("metadata"))
                })
                .map(|f| {
                    let mut f = f.clone();
                    if let Some(obj) = f.as_object_mut() {
                        strip_str_field(obj, "function_id", session);
                        strip_str_field(obj, "worker_name", session);
                    }
                    f
                })
                .collect()
        })
        .unwrap_or_default();
    result["functions"] = Value::Array(kept);
    OverrideOutcome::Result(result)
}

fn functions_info(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
) -> OverrideOutcome {
    let id = result
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !allowed(cfg, session, &id, result.get("metadata")) {
        return OverrideOutcome::forbidden(&id);
    }
    if let Some(obj) = result.as_object_mut() {
        strip_str_field(obj, "function_id", session);
        strip_str_field(obj, "worker_name", session);
    }
    // The function's `registered_triggers` all target this (allowed) function,
    // so there is nothing to drop; the ref carries no foreign function id.
    OverrideOutcome::Result(result)
}

async fn workers_list(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    let workers = result
        .get("workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut kept = Vec::new();
    for w in workers {
        let name = w.get("name").and_then(Value::as_str).unwrap_or("");
        // Resolve the worker's function set via the catalog (the response has
        // only a count, no ids).
        let fns = catalog.functions_of_worker(name).await;
        let accessible = fns
            .iter()
            .filter(|(id, md)| allowed(cfg, session, id, md.as_ref()))
            .count();
        if accessible == 0 {
            continue; // drop workers with zero accessible functions
        }
        let mut w = w;
        if let Some(obj) = w.as_object_mut() {
            obj.insert("function_count".to_string(), json!(accessible));
            strip_worker_summary_internals(obj, cfg);
        }
        kept.push(w);
    }
    result["workers"] = Value::Array(kept);
    OverrideOutcome::Result(result)
}

async fn workers_info(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    // Extract the arrays as owned values up front so the async filtering below
    // does not hold a borrow of `result` across the later mutable access.
    let funcs_in = result
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let regs_in = result
        .get("registered_triggers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // functions[]: this response carries them (with metadata) directly.
    let funcs: Vec<Value> = funcs_in
        .into_iter()
        .filter(|f| {
            let id = f.get("function_id").and_then(Value::as_str).unwrap_or("");
            allowed(cfg, session, id, f.get("metadata"))
        })
        .map(|mut f| {
            if let Some(o) = f.as_object_mut() {
                strip_str_field(o, "function_id", session);
                strip_str_field(o, "worker_name", session);
            }
            f
        })
        .collect();

    if funcs.is_empty() {
        return OverrideOutcome::not_found("worker not found");
    }

    // registered_triggers[]: carry function_id → drop denied, strip prefix.
    let regs: Vec<Value> = filter_registered_triggers(&regs_in, session, cfg, catalog).await;

    let count = funcs.len();
    if let Some(obj) = result.as_object_mut() {
        obj.insert("functions".to_string(), Value::Array(funcs));
        obj.insert("registered_triggers".to_string(), Value::Array(regs));
        // trigger_types[] are capability metadata (no function binding) → kept.
        if let Some(worker) = obj.get_mut("worker").and_then(Value::as_object_mut) {
            worker.insert("function_count".to_string(), json!(count));
            strip_worker_detail_internals(worker, cfg);
        }
    }
    OverrideOutcome::Result(result)
}

async fn triggers_info(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    // Recompute instance_count to the accessible subset so it cannot leak how
    // many *hidden* functions use the type. Schemas are kept.
    let type_id = result
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let targets = catalog.functions_for_trigger_type(&type_id).await;
    let mut accessible = 0usize;
    for fid in &targets {
        if allowed_via_cache(cfg, session, fid, catalog).await {
            accessible += 1;
        }
    }
    // Only override the count when we actually have binding data; otherwise
    // leave the engine's value (cold cache) rather than wrongly zeroing it.
    if !targets.is_empty() {
        result["instance_count"] = json!(accessible);
    }
    OverrideOutcome::Result(result)
}

async fn registered_triggers_list(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    let arr = result
        .get("registered_triggers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let kept = filter_registered_triggers(&arr, session, cfg, catalog).await;
    result["registered_triggers"] = Value::Array(kept);
    OverrideOutcome::Result(result)
}

async fn registered_triggers_info(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> OverrideOutcome {
    let fid = result
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !allowed_via_cache(cfg, session, &fid, catalog).await {
        return OverrideOutcome::forbidden(&fid);
    }
    if let Some(obj) = result.as_object_mut() {
        strip_str_field(obj, "function_id", session);
        // Null out the nested `function` envelope if it somehow references a
        // denied function (defensive — it references this allowed function).
        if let Some(func) = obj.get("function") {
            if !func.is_null() {
                let nested_id = func
                    .get("function_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let nested_md = func.get("metadata");
                if !allowed(cfg, session, nested_id, nested_md) {
                    obj.insert("function".to_string(), Value::Null);
                } else if let Some(f) = obj.get_mut("function").and_then(Value::as_object_mut) {
                    strip_str_field(f, "function_id", session);
                    strip_str_field(f, "worker_name", session);
                }
            }
        }
    }
    OverrideOutcome::Result(result)
}

/// Drop registered-trigger entries whose target function fails `A`, and strip
/// the session prefix from the surviving `function_id`s.
async fn filter_registered_triggers(
    arr: &[Value],
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
) -> Vec<Value> {
    let mut out = Vec::new();
    for t in arr {
        let fid = t.get("function_id").and_then(Value::as_str).unwrap_or("");
        if !allowed_via_cache(cfg, session, fid, catalog).await {
            continue;
        }
        let mut t = t.clone();
        if let Some(obj) = t.as_object_mut() {
            strip_str_field(obj, "function_id", session);
        }
        out.push(t);
    }
    out
}

/// `WorkerSummary` (engine::workers::list) carries only `ip_address` and
/// `isolation` as operational internals.
fn strip_worker_summary_internals(obj: &mut serde_json::Map<String, Value>, cfg: &WorkerConfig) {
    if cfg.expose_worker_internals {
        return;
    }
    obj.remove("ip_address");
    obj.remove("isolation");
}

/// `WorkerDetailEnvelope` (engine::workers::info `worker`) adds `pid`,
/// `internal`, and `latest_metrics`.
fn strip_worker_detail_internals(obj: &mut serde_json::Map<String, Value>, cfg: &WorkerConfig) {
    if cfg.expose_worker_internals {
        return;
    }
    obj.remove("ip_address");
    obj.remove("isolation");
    obj.remove("pid");
    obj.remove("internal");
    obj.remove("latest_metrics");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rbac::{FunctionFilter, RbacConfig, WildcardPattern};

    fn iii() -> Arc<IIIClient> {
        Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ))
    }

    fn cfg(patterns: &[&str], expose_internals: bool) -> WorkerConfig {
        WorkerConfig {
            expose_worker_internals: expose_internals,
            rbac: RbacConfig {
                expose_functions: patterns
                    .iter()
                    .map(|p| FunctionFilter::Match(WildcardPattern::new(p)))
                    .collect(),
                ..RbacConfig::default()
            },
            ..WorkerConfig::default()
        }
    }

    fn session(prefix: Option<&str>) -> ProxySession {
        ProxySession {
            function_registration_prefix: prefix.map(str::to_string),
            ..ProxySession::permissive("ip".to_string())
        }
    }

    #[tokio::test]
    async fn functions_list_keeps_only_allowed_and_strips_prefix() {
        let result = json!({
            "functions": [
                { "function_id": "api::users::list", "worker_name": "api", "metadata": null },
                { "function_id": "secret::do", "worker_name": "sec" },
                { "function_id": "tenant1::foo", "worker_name": "w" }
            ]
        });
        // expose api::* and the caller's own tenant1::* (so its prefixed fn
        // survives and is stripped to bare).
        let c = cfg(&["api::*", "tenant1::*"], false);
        let out = filter_result(
            "engine::functions::list",
            result,
            &session(Some("tenant1")),
            &c,
            &CatalogCache::new(iii()),
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        let ids: Vec<&str> = v["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["function_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["api::users::list", "foo"]); // secret::do dropped, tenant1::foo → foo
    }

    #[tokio::test]
    async fn functions_info_forbidden_when_denied() {
        let result =
            json!({ "function_id": "secret::do", "worker_name": "sec", "registered_triggers": [] });
        let out = filter_result(
            "engine::functions::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert_eq!(
            out,
            OverrideOutcome::Error {
                code: "FORBIDDEN".to_string(),
                message: "function 'secret::do' not allowed (add to rbac.expose_functions)"
                    .to_string()
            }
        );
    }

    #[tokio::test]
    async fn functions_info_allows_and_strips() {
        let result =
            json!({ "function_id": "tenant1::foo", "worker_name": "w", "registered_triggers": [] });
        let out = filter_result(
            "engine::functions::info",
            result,
            &session(Some("tenant1")),
            &cfg(&["tenant1::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        assert_eq!(v["function_id"], "foo");
    }

    #[tokio::test]
    async fn workers_list_drops_empty_workers_recounts_and_strips_internals() {
        let cache = CatalogCache::new(iii());
        cache
            .seed_functions(&[
                ("api::a", "api-worker", None),
                ("api::b", "api-worker", None),
                ("secret::z", "secret-worker", None),
            ])
            .await;
        let result = json!({
            "workers": [
                { "name": "api-worker", "id": "1", "status": "connected", "function_count": 2, "connected_at_ms": 0, "active_invocations": 0, "ip_address": "10.0.0.9", "isolation": "vm" },
                { "name": "secret-worker", "id": "2", "status": "connected", "function_count": 1, "connected_at_ms": 0, "active_invocations": 0, "ip_address": "10.0.0.8" }
            ]
        });
        let out = filter_result(
            "engine::workers::list",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &cache,
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        let workers = v["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 1, "secret-worker has 0 accessible fns");
        assert_eq!(workers[0]["name"], "api-worker");
        assert_eq!(workers[0]["function_count"], 2);
        assert!(workers[0].get("ip_address").is_none(), "internal stripped");
        assert!(workers[0].get("isolation").is_none(), "internal stripped");
    }

    #[tokio::test]
    async fn workers_list_keeps_internals_when_exposed() {
        let cache = CatalogCache::new(iii());
        cache
            .seed_functions(&[("api::a", "api-worker", None)])
            .await;
        let result = json!({
            "workers": [ { "name": "api-worker", "id": "1", "status": "connected", "function_count": 1, "connected_at_ms": 0, "active_invocations": 0, "ip_address": "10.0.0.9", "isolation": "vm" } ]
        });
        let out = filter_result(
            "engine::workers::list",
            result,
            &session(None),
            &cfg(&["api::*"], true),
            &cache,
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        assert_eq!(v["workers"][0]["ip_address"], "10.0.0.9");
        assert_eq!(v["workers"][0]["isolation"], "vm");
    }

    #[tokio::test]
    async fn workers_info_filters_functions_and_strips_envelope() {
        let result = json!({
            "worker": { "name": "api-worker", "id": "1", "status": "connected", "function_count": 2, "connected_at_ms": 0, "active_invocations": 0, "ip_address": "10.0.0.1", "isolation": "vm", "pid": 42, "internal": false, "latest_metrics": { "timestamp_ms": 1, "runtime": "rust" } },
            "functions": [
                { "function_id": "api::a", "worker_name": "api-worker" },
                { "function_id": "secret::z", "worker_name": "api-worker" }
            ],
            "trigger_types": [ { "id": "cron", "worker_name": "api-worker", "description": "d" } ],
            "registered_triggers": [
                { "id": "t1", "trigger_type": "cron", "function_id": "api::a", "worker_name": "api-worker", "config": {}, "config_summary": "" },
                { "id": "t2", "trigger_type": "cron", "function_id": "secret::z", "worker_name": "api-worker", "config": {}, "config_summary": "" }
            ]
        });
        let out = filter_result(
            "engine::workers::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        assert_eq!(v["functions"].as_array().unwrap().len(), 1);
        assert_eq!(v["functions"][0]["function_id"], "api::a");
        assert_eq!(v["registered_triggers"].as_array().unwrap().len(), 1);
        assert_eq!(v["worker"]["function_count"], 1);
        assert!(v["worker"].get("pid").is_none());
        assert!(v["worker"].get("ip_address").is_none());
        assert!(v["worker"].get("latest_metrics").is_none());
        assert!(v["worker"].get("internal").is_none());
    }

    #[tokio::test]
    async fn workers_info_not_found_when_no_accessible_functions() {
        let result = json!({
            "worker": { "name": "secret-worker", "id": "1", "status": "connected", "function_count": 1, "connected_at_ms": 0, "active_invocations": 0 },
            "functions": [ { "function_id": "secret::z", "worker_name": "secret-worker" } ],
            "trigger_types": [],
            "registered_triggers": []
        });
        let out = filter_result(
            "engine::workers::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert!(matches!(out, OverrideOutcome::Error { code, .. } if code == "NOT_FOUND"));
    }

    #[tokio::test]
    async fn registered_triggers_list_drops_denied_and_strips() {
        let result = json!({
            "registered_triggers": [
                { "id": "t1", "trigger_type": "cron", "function_id": "api::a", "worker_name": "w", "config": {}, "config_summary": "" },
                { "id": "t2", "trigger_type": "cron", "function_id": "secret::z", "worker_name": "w", "config": {}, "config_summary": "" },
                { "id": "t3", "trigger_type": "cron", "function_id": "tenant1::foo", "worker_name": "w", "config": {}, "config_summary": "" }
            ]
        });
        let out = filter_result(
            "engine::registered-triggers::list",
            result,
            &session(Some("tenant1")),
            &cfg(&["api::*", "tenant1::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        let arr = v["registered_triggers"].as_array().unwrap();
        let fids: Vec<&str> = arr
            .iter()
            .map(|t| t["function_id"].as_str().unwrap())
            .collect();
        assert_eq!(fids, vec!["api::a", "foo"]); // secret::z dropped, tenant1::foo → foo
    }

    #[tokio::test]
    async fn triggers_info_recomputes_instance_count_to_accessible() {
        let cache = CatalogCache::new(iii());
        cache
            .seed_bindings(&[("cron", &["api::a", "secret::z", "api::b"])])
            .await;
        let result =
            json!({ "id": "cron", "worker_name": "w", "description": "d", "instance_count": 3 });
        let out = filter_result(
            "engine::triggers::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &cache,
        )
        .await;
        let OverrideOutcome::Result(v) = out else {
            panic!("expected result");
        };
        assert_eq!(v["instance_count"], 2, "only api::a and api::b accessible");
    }

    #[tokio::test]
    async fn triggers_list_passes_through() {
        let result =
            json!({ "triggers": [ { "id": "cron", "worker_name": "w", "description": "d" } ] });
        let out = filter_result(
            "engine::triggers::list",
            result.clone(),
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert_eq!(out, OverrideOutcome::Result(result));
    }
}
