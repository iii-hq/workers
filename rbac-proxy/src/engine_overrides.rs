//! Engine function overrides — the addition over the engine's RBAC contract.
//!
//! An engine RBAC listener gates *invocation* but only partially filters
//! *discovery*: a gated worker that can reach the discovery functions can still
//! enumerate functions, workers, and observers it can never call. `rbac-proxy`
//! closes that gap by **result-filtering the six discovery functions** to the
//! caller's boundaries, using the **same** vendored [`rbac::access_allowed`]
//! predicate as the invocation path — so the discovery surface and the
//! invocation surface can never disagree.
//!
//! `engine::workers::list` cannot be filtered from its own response alone (it
//! carries only a `function_count`, no ids), and observer rows carry no
//! function metadata, so this module also keeps a small TTL cache of the
//! function catalog (`function_id → { worker_name, metadata }`) over the
//! proxy's **control connection**.
//!
//! Results are rewritten as [`serde_json::Value`] so unknown fields survive —
//! the proxy only removes/edits the keys it must.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::config::WorkerConfig;
use crate::rbac::{self, ProxySession, DEFAULT_NAMESPACE};

/// The six discovery functions whose results are rewritten.
/// `engine::workers::register` is a carve-out write, not a discovery surface,
/// and is never rewritten; `engine::observers::dispose` is gated by the
/// interceptor (only observers the connection created).
pub const DISCOVERY_FUNCTIONS: &[&str] = &[
    "engine::functions::list",
    "engine::functions::info",
    "engine::workers::list",
    "engine::workers::info",
    "engine::observers::list",
    "engine::observers::get",
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

    /// Engine parity: `engine::observers::get` answers an observer the caller
    /// may not see as if it did not exist.
    fn observer_not_found(id: &str) -> Self {
        OverrideOutcome::Error {
            code: "OBSERVER_NOT_FOUND".to_string(),
            message: format!("no observer `{id}`"),
        }
    }
}

// ---------------------------------------------------------------------------
// Catalog cache
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FnEntry {
    worker_name: String,
    metadata: Option<Value>,
}

/// `(namespace, function_id)`: the key a function lives under in the engine.
type FnKey = (String, String);

#[derive(Default)]
struct FnState {
    entries: HashMap<FnKey, FnEntry>,
    fetched_at: Option<Instant>,
}

/// Short TTL — the discovery surface tolerates seconds of staleness, and a
/// metadata-gated function is briefly invisible (fail closed) until refresh.
/// `engine::on-functions-changed` invalidates the cache sooner; the TTL bounds
/// staleness for whatever that event misses.
const CACHE_TTL: Duration = Duration::from_secs(5);

/// TTL cache over the control connection. Keyed by the **engine** id
/// (prefixed where applicable, since that is what discovery results carry)
/// and by namespace, `(namespace, function_id)`: the same id may be
/// registered once per namespace.
pub struct CatalogCache {
    iii: Arc<IIIClient>,
    functions: RwLock<FnState>,
    /// Serializes refreshes so a cold cache does not stampede the engine.
    refresh: Mutex<()>,
}

impl CatalogCache {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            functions: RwLock::new(FnState::default()),
            refresh: Mutex::new(()),
        }
    }

    /// Force a refresh on the next access. Called from the
    /// `rbac-proxy::on-functions-changed` handler; `CACHE_TTL` still bounds
    /// staleness for whatever that event misses.
    pub async fn invalidate(&self) {
        self.functions.write().await.fetched_at = None;
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

    async fn fetch_functions(&self) -> Result<HashMap<FnKey, FnEntry>, String> {
        let resp = self
            .iii
            .call(CallRequest::new("engine::functions::list", json!({})))
            .await
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        if let Some(arr) = resp.get("functions").and_then(Value::as_array) {
            for f in arr {
                let Some(id) = f.get("function_id").and_then(Value::as_str) else {
                    continue;
                };
                out.insert(
                    (entry_namespace(f).to_string(), id.to_string()),
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

    /// The registered metadata of `function_id` in `namespace` (None on a cold
    /// cache → fail closed for metadata filters, open for wildcard filters
    /// which ignore metadata).
    pub async fn metadata_for(&self, namespace: &str, function_id: &str) -> Option<Value> {
        self.ensure_functions().await;
        self.functions
            .read()
            .await
            .entries
            .get(&(namespace.to_string(), function_id.to_string()))
            .and_then(|e| e.metadata.clone())
    }

    /// All `(namespace, function_id, metadata)` owned by `worker_name`.
    async fn functions_of_worker(&self, worker_name: &str) -> Vec<(String, String, Option<Value>)> {
        self.ensure_functions().await;
        self.functions
            .read()
            .await
            .entries
            .iter()
            .filter(|(_, e)| e.worker_name == worker_name)
            .map(|((ns, id), e)| (ns.clone(), id.clone(), e.metadata.clone()))
            .collect()
    }

    /// Seed `(function_id, worker_name, metadata)` entries in `default`.
    #[cfg(test)]
    pub async fn seed_functions(&self, entries: &[(&str, &str, Option<Value>)]) {
        let mut st = self.functions.write().await;
        st.entries = entries
            .iter()
            .map(|(id, wn, md)| {
                (
                    (DEFAULT_NAMESPACE.to_string(), id.to_string()),
                    FnEntry {
                        worker_name: wn.to_string(),
                        metadata: md.clone(),
                    },
                )
            })
            .collect();
        st.fetched_at = Some(Instant::now());
    }
}

// ---------------------------------------------------------------------------
// The per-function rewrite table
// ---------------------------------------------------------------------------

/// Filter a discovery result to the caller's boundaries. `function_id` is the
/// discovery function being answered; `result` is the engine's full,
/// unfiltered result. `own_observers` are the ids of the observers this
/// connection created, which it always sees.
pub async fn filter_result(
    function_id: &str,
    result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
    own_observers: &HashSet<String>,
) -> OverrideOutcome {
    match function_id {
        "engine::functions::list" => functions_list(result, session, cfg),
        "engine::functions::info" => functions_info(result, session, cfg),
        "engine::workers::list" => workers_list(result, session, cfg, catalog).await,
        "engine::workers::info" => workers_info(result, session, cfg),
        "engine::observers::list" => {
            observers_list(result, session, cfg, catalog, own_observers).await
        }
        "engine::observers::get" => {
            observers_get(result, session, cfg, catalog, own_observers).await
        }
        _ => OverrideOutcome::Result(result),
    }
}

/// The namespace a discovery entry names, `default` for engines that predate
/// namespaces (and for entries that carry none).
fn entry_namespace(entry: &Value) -> &str {
    entry
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_NAMESPACE)
}

/// `A(namespace, id, metadata)` against the live rbac config + this session.
fn allowed(
    cfg: &WorkerConfig,
    session: &ProxySession,
    namespace: &str,
    id: &str,
    metadata: Option<&Value>,
) -> bool {
    rbac::access_allowed(Some(&cfg.rbac), session, namespace, id, metadata)
}

/// `A` for a discovery entry that carries its own `function_id`, `namespace`
/// and `metadata` (functions list/info rows).
fn entry_allowed(cfg: &WorkerConfig, session: &ProxySession, entry: &Value) -> bool {
    let id = entry
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    allowed(
        cfg,
        session,
        entry_namespace(entry),
        id,
        entry.get("metadata"),
    )
}

/// `A(namespace, id)` for an id whose metadata is not in the result (the
/// observed function and the target of an observer row). Consults the
/// catalog **only** when a metadata filter is configured — wildcard-only
/// configs need no round trip and a cold cache must not fail them closed.
async fn allowed_via_cache(
    cfg: &WorkerConfig,
    session: &ProxySession,
    namespace: &str,
    id: &str,
    catalog: &CatalogCache,
) -> bool {
    let md = if cfg.rbac.uses_metadata() {
        catalog.metadata_for(namespace, id).await
    } else {
        None
    };
    allowed(cfg, session, namespace, id, md.as_ref())
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
                .filter(|f| entry_allowed(cfg, session, f))
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
    // `{ function_ids: [...] }` answers with a batch envelope: one entry per id,
    // each the detail or `{ function_id, error }`. A denied detail becomes the
    // engine's own per-id `forbidden` entry, so the batch keeps its shape.
    if result.get("function_id").is_none() {
        if let Some(entries) = result.get("functions").and_then(Value::as_array) {
            let entries: Vec<Value> = entries
                .iter()
                .map(|entry| {
                    let id = entry
                        .get("function_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if entry.get("error").is_some() {
                        let mut entry = entry.clone();
                        if let Some(obj) = entry.as_object_mut() {
                            strip_str_field(obj, "function_id", session);
                        }
                        entry
                    } else if !entry_allowed(cfg, session, entry) {
                        json!({ "function_id": strip(session, id), "error": "forbidden" })
                    } else {
                        let mut entry = entry.clone();
                        if let Some(obj) = entry.as_object_mut() {
                            strip_str_field(obj, "function_id", session);
                            strip_str_field(obj, "worker_name", session);
                        }
                        entry
                    }
                })
                .collect();
            result["functions"] = Value::Array(entries);
            return OverrideOutcome::Result(result);
        }
    }

    let id = result
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !entry_allowed(cfg, session, &result) {
        return OverrideOutcome::forbidden(&id);
    }
    if let Some(obj) = result.as_object_mut() {
        strip_str_field(obj, "function_id", session);
        strip_str_field(obj, "worker_name", session);
    }
    // The row names no other function (`observer_count` is a count), so
    // there is nothing more to drop.
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
            .filter(|(ns, id, md)| allowed(cfg, session, ns, id, md.as_ref()))
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

fn workers_info(mut result: Value, session: &ProxySession, cfg: &WorkerConfig) -> OverrideOutcome {
    let funcs_in = result
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // functions[]: this response carries them (with metadata) directly.
    let funcs: Vec<Value> = funcs_in
        .into_iter()
        .filter(|f| entry_allowed(cfg, session, f))
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

    let count = funcs.len();
    if let Some(obj) = result.as_object_mut() {
        obj.insert("functions".to_string(), Value::Array(funcs));
        // `observers[]` are the ids of the observers the worker created:
        // opaque, and `engine::observers::get` filters each one.
        if let Some(worker) = obj.get_mut("worker").and_then(Value::as_object_mut) {
            worker.insert("function_count".to_string(), json!(count));
            strip_worker_detail_internals(worker, cfg);
        }
    }
    OverrideOutcome::Result(result)
}

/// An observer row (`ObserverInfo`) the caller may see: one it created, or
/// one whose observed function and target it may both invoke.
async fn observer_visible(
    row: &Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
    own_observers: &HashSet<String>,
) -> bool {
    if row
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| own_observers.contains(id))
    {
        return true;
    }
    let function_id = row.get("function_id").and_then(Value::as_str).unwrap_or("");
    let target = row.get("target").unwrap_or(&Value::Null);
    let target_id = target
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    allowed_via_cache(cfg, session, entry_namespace(row), function_id, catalog).await
        && allowed_via_cache(cfg, session, entry_namespace(target), target_id, catalog).await
}

/// Strip the session prefix from an observer row's function and target ids.
fn strip_observer_row(row: &mut Value, session: &ProxySession) {
    if let Some(obj) = row.as_object_mut() {
        strip_str_field(obj, "function_id", session);
        if let Some(target) = obj.get_mut("target").and_then(Value::as_object_mut) {
            strip_str_field(target, "function_id", session);
        }
    }
}

async fn observers_list(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
    own_observers: &HashSet<String>,
) -> OverrideOutcome {
    let rows = result
        .get("observers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut kept = Vec::new();
    for mut row in rows {
        if !observer_visible(&row, session, cfg, catalog, own_observers).await {
            continue;
        }
        strip_observer_row(&mut row, session);
        kept.push(row);
    }
    result["observers"] = Value::Array(kept);
    OverrideOutcome::Result(result)
}

async fn observers_get(
    mut result: Value,
    session: &ProxySession,
    cfg: &WorkerConfig,
    catalog: &CatalogCache,
    own_observers: &HashSet<String>,
) -> OverrideOutcome {
    if !observer_visible(&result, session, cfg, catalog, own_observers).await {
        let id = result.get("id").and_then(Value::as_str).unwrap_or("");
        return OverrideOutcome::observer_not_found(id);
    }
    strip_observer_row(&mut result, session);
    OverrideOutcome::Result(result)
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
    use crate::rbac::{FunctionFilter, RbacConfig};

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
                    .map(|p| FunctionFilter::match_pattern(p))
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

    /// `filter_result` for a connection that created no observers.
    async fn filter(
        function_id: &str,
        result: Value,
        session: &ProxySession,
        cfg: &WorkerConfig,
        catalog: &CatalogCache,
    ) -> OverrideOutcome {
        filter_result(function_id, result, session, cfg, catalog, &HashSet::new()).await
    }

    #[tokio::test]
    async fn functions_list_keeps_only_allowed_and_strips_prefix() {
        let result = json!({
            "functions": [
                { "function_id": "api::users::list", "worker_name": "api", "metadata": null, "observer_count": 0 },
                { "function_id": "secret::do", "worker_name": "sec", "observer_count": 2 },
                { "function_id": "tenant1::foo", "worker_name": "w", "observer_count": 0 }
            ]
        });
        // expose api::* and the caller's own tenant1::* (so its prefixed fn
        // survives and is stripped to bare).
        let c = cfg(&["api::*", "tenant1::*"], false);
        let out = filter(
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
            json!({ "function_id": "secret::do", "worker_name": "sec", "observer_count": 0 });
        let out = filter(
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
        let result = json!({
            "function_id": "tenant1::foo",
            "worker_name": "w",
            "private": true,
            "args_format": { "type": "object" },
            "observer_count": 1
        });
        let out = filter(
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
        assert_eq!(v["private"], true);
        assert_eq!(v["observer_count"], 1);
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
        let out = filter(
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
        let out = filter(
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
                { "function_id": "api::a", "worker_name": "api-worker", "observer_count": 0 },
                { "function_id": "secret::z", "worker_name": "api-worker", "observer_count": 1 }
            ],
            "observers": ["5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d"]
        });
        let out = filter(
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
        assert_eq!(
            v["observers"],
            json!(["5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d"])
        );
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
            "observers": []
        });
        let out = filter(
            "engine::workers::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert!(matches!(out, OverrideOutcome::Error { code, .. } if code == "NOT_FOUND"));
    }

    /// An `ObserverInfo` row as `engine::observers::list` reports it.
    fn observer_row(id: &str, function_id: &str, target: &str) -> Value {
        json!({
            "id": id,
            "function_id": function_id,
            "namespace": "default",
            "args": {},
            "filters": {},
            "target": { "function_id": target, "namespace": "default" },
            "capture": ["input", "output", "caller"],
            "on": "success",
            "deliver": "envelope",
            "worker": { "worker_id": "7c1e0b52-93d4-4f0e-8a3b-6d2f1c9e4a70", "worker_name": "w" },
            "state": "active",
            "fires": 0,
            "dropped": {},
            "activation_failures": {},
            "max_in_flight": 64,
            "created_at_ms": 0
        })
    }

    #[tokio::test]
    async fn observers_list_keeps_rows_whose_function_and_target_are_allowed() {
        let result = json!({
            "observers": [
                observer_row("o-1", "api::orders", "api::sink"),
                observer_row("o-2", "secret::z", "api::sink"),
                observer_row("o-3", "api::orders", "secret::sink"),
                observer_row("o-4", "api::orders", "tenant1::handler"),
            ]
        });
        let OverrideOutcome::Result(v) = filter(
            "engine::observers::list",
            result,
            &session(Some("tenant1")),
            &cfg(&["api::*", "tenant1::*"], false),
            &CatalogCache::new(iii()),
        )
        .await
        else {
            panic!("expected result");
        };
        let rows: Vec<(&str, &str)> = v["observers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["id"].as_str().unwrap(),
                    o["target"]["function_id"].as_str().unwrap(),
                )
            })
            .collect();
        // o-2 observes a hidden function and o-3 targets one; o-4's own
        // target is stripped to the bare id the session registered.
        assert_eq!(rows, vec![("o-1", "api::sink"), ("o-4", "handler")]);
    }

    #[tokio::test]
    async fn a_connection_sees_the_observers_it_created() {
        let result =
            json!({ "observers": [observer_row("mine", "secret::z", "tenant1::handler")] });
        let own: HashSet<String> = ["mine".to_string()].into_iter().collect();
        let OverrideOutcome::Result(v) = filter_result(
            "engine::observers::list",
            result,
            &session(Some("tenant1")),
            &cfg(&[], false),
            &CatalogCache::new(iii()),
            &own,
        )
        .await
        else {
            panic!("expected result");
        };
        assert_eq!(v["observers"][0]["id"], "mine");
        assert_eq!(v["observers"][0]["target"]["function_id"], "handler");
    }

    #[tokio::test]
    async fn observers_get_hides_an_observer_the_caller_may_not_see() {
        let out = filter(
            "engine::observers::get",
            observer_row("o-2", "secret::z", "api::sink"),
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert_eq!(
            out,
            OverrideOutcome::Error {
                code: "OBSERVER_NOT_FOUND".to_string(),
                message: "no observer `o-2`".to_string()
            }
        );

        let OverrideOutcome::Result(v) = filter(
            "engine::observers::get",
            observer_row("o-1", "api::orders", "api::sink"),
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await
        else {
            panic!("expected result");
        };
        assert_eq!(v["id"], "o-1");
    }

    #[tokio::test]
    async fn an_observer_row_is_checked_in_its_namespaces() {
        let mut row = observer_row("o-1", "api::orders", "api::sink");
        row["target"]["namespace"] = json!("orders");
        let out = filter(
            "engine::observers::get",
            row,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert!(matches!(out, OverrideOutcome::Error { code, .. } if code == "OBSERVER_NOT_FOUND"));
    }

    #[tokio::test]
    async fn functions_list_checks_each_row_in_its_own_namespace() {
        // The same id in two namespaces; `api::*` (no namespace) exposes only
        // the `default` row, and a rule scoped to `orders` only the other.
        let result = json!({
            "functions": [
                { "function_id": "api::a", "namespace": "default", "worker_name": "w" },
                { "function_id": "api::a", "namespace": "orders", "worker_name": "w" },
                { "function_id": "billing::x", "namespace": "billing", "worker_name": "w" }
            ]
        });
        let mut c = cfg(&["api::*"], false);
        c.rbac
            .expose_functions
            .push(FunctionFilter::match_pattern("billing::*").in_namespace("billing"));
        let OverrideOutcome::Result(v) = filter(
            "engine::functions::list",
            result,
            &session(None),
            &c,
            &CatalogCache::new(iii()),
        )
        .await
        else {
            panic!("expected result");
        };
        let rows: Vec<(&str, &str)> = v["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["namespace"].as_str().unwrap(),
                    f["function_id"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(rows, vec![("default", "api::a"), ("billing", "billing::x")]);
    }

    #[tokio::test]
    async fn functions_info_denies_a_function_outside_the_rule_namespace() {
        let result = json!({ "function_id": "api::a", "namespace": "orders", "worker_name": "w" });
        let out = filter(
            "engine::functions::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await;
        assert!(matches!(out, OverrideOutcome::Error { code, .. } if code == "FORBIDDEN"));
    }

    #[tokio::test]
    async fn functions_info_batch_marks_denied_entries_forbidden() {
        let result = json!({
            "functions": [
                { "function_id": "api::a", "namespace": "default", "worker_name": "w", "observer_count": 0 },
                { "function_id": "secret::z", "namespace": "default", "worker_name": "w", "observer_count": 0 },
                { "function_id": "gone::x", "error": "not_found" }
            ]
        });
        let OverrideOutcome::Result(v) = filter(
            "engine::functions::info",
            result,
            &session(None),
            &cfg(&["api::*"], false),
            &CatalogCache::new(iii()),
        )
        .await
        else {
            panic!("expected result");
        };
        let entries = v["functions"].as_array().unwrap();
        assert_eq!(entries[0]["function_id"], "api::a");
        assert!(entries[0].get("error").is_none());
        assert_eq!(
            entries[1],
            json!({ "function_id": "secret::z", "error": "forbidden" })
        );
        assert_eq!(entries[2]["error"], "not_found");
    }

    #[tokio::test]
    async fn metadata_lookup_is_keyed_by_namespace() {
        let cache = CatalogCache::new(iii());
        cache
            .seed_functions(&[("api::a", "w", Some(json!({ "public": true })))])
            .await;
        assert_eq!(
            cache.metadata_for(DEFAULT_NAMESPACE, "api::a").await,
            Some(json!({ "public": true }))
        );
        assert_eq!(cache.metadata_for("orders", "api::a").await, None);
    }
}
