//! `directory::registry::*` — HTTP proxy over
//! `https://api.workers.iii.dev`.
//!
//! Two functions, mirroring `directory::engine::workers::*` so callers
//! learn one shape:
//!
//!   * `directory::registry::workers::list`  — list workers in the
//!     public registry, filterable by `search`. Same row envelope
//!     (`Worker`) as [`crate::functions::directory::Worker`].
//!   * `directory::registry::workers::info`  — full registry metadata
//!     for one worker. Wraps the registry-side fields in a top-level
//!     `worker` envelope (same shape as the list rows), with `readme`
//!     / `api_reference` / `skills_tree` as surface-specific extras.
//!
//! Both responses are cached in-process for `registry_cache_ttl_ms`
//! (default 60s) so repeat lookups don't hammer the registry — every
//! call is a separate HTTP request without it.
//!
//! HTTP shapes assumed:
//!
//!   * `GET {base}/search?q=…&limit=…` → `{ workers: [...] }`
//!   * `GET {base}/w/{worker}?version=…|tag=…` → flat envelope mirroring
//!     the publish payload (`name`, `version`, `description`, `repo`,
//!     `author`, `readme`, `functions`, `triggers`, `skills_tree`).
//!
//! If either path turns out to differ on the live registry, only the
//! `fetch_*` helpers below need adjusting; the public function
//! input/output contracts stay stable.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::config::SkillsConfig;
use crate::sources::build_http_client;

const SEARCH_LIMIT_DEFAULT: u32 = 20;
const SEARCH_LIMIT_MAX: u32 = 100;

// ---------- public input/output shapes ----------

/// `directory::registry::workers::list` input. Mirrors
/// [`crate::functions::directory::WorkerListInput.search`] so callers
/// can switch between local and registry surfaces without re-learning
/// the API. Adds `limit` for paging because the registry is paged.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkerListInput {
    /// Free-text query forwarded to the registry as `?q=…`. Required —
    /// the public registry doesn't support an unscoped browse endpoint.
    #[serde(default)]
    pub search: Option<String>,
    /// Max results to return. Defaults to 20; capped at 100.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct RegistryAuthor {
    pub name: Option<String>,
    pub profile_picture: Option<String>,
    #[serde(default)]
    pub is_verified: bool,
}

/// Shared worker envelope used by both
/// `directory::registry::workers::list` rows and the `worker` field of
/// `directory::registry::workers::info`. Same field names as
/// [`crate::functions::directory::Worker`] so callers learn one shape
/// across local + registry surfaces.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Worker {
    pub name: String,
    pub description: Option<String>,
    /// Latest published version (worker-list) or the resolved version
    /// (worker-info, when called with `version` / `tag`).
    pub version: Option<String>,
    pub repo: Option<String>,
    #[serde(default)]
    pub author: Option<RegistryAuthor>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkerListOutput {
    pub workers: Vec<Worker>,
}

/// `directory::registry::workers::info` input. Pass either `version`
/// or `tag`; if neither is provided we fall back to `tag: "latest"`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkerInfoInput {
    /// Worker name in the registry (e.g. `"resend"`).
    pub name: String,
    /// Mutually exclusive with `tag`. If neither is provided we fall back
    /// to `tag: "latest"`.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ApiReferenceFunction {
    pub name: String,
    pub description: Option<String>,
    pub request_schema: Option<Value>,
    pub response_schema: Option<Value>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct ApiReferenceTrigger {
    pub name: String,
    pub description: Option<String>,
    pub invocation_schema: Option<Value>,
    pub return_schema: Option<Value>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
pub struct ApiReference {
    #[serde(default)]
    pub functions: Vec<ApiReferenceFunction>,
    #[serde(default)]
    pub triggers: Vec<ApiReferenceTrigger>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct SkillsTreeSkill {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct SkillsTreePrompt {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
pub struct SkillsTree {
    #[serde(default)]
    pub skills: Vec<SkillsTreeSkill>,
    #[serde(default)]
    pub prompts: Vec<SkillsTreePrompt>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkerInfoOutput {
    /// Same shape as `directory::registry::workers::list` rows (and
    /// `directory::engine::workers::info.worker`).
    pub worker: Worker,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    pub api_reference: ApiReference,
    pub skills_tree: SkillsTree,
}

// ---------- shared cache + http client ----------

#[derive(Clone)]
pub struct RegistryCache {
    inner: Arc<RwLock<std::collections::HashMap<String, CacheEntry>>>,
    ttl: Duration,
}

struct CacheEntry {
    value: serde_json::Value,
    inserted_at: Instant,
}

impl RegistryCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(std::collections::HashMap::new())),
            ttl,
        }
    }

    pub async fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let map = self.inner.read().await;
        let entry = map.get(key)?;
        if entry.inserted_at.elapsed() > self.ttl {
            return None;
        }
        serde_json::from_value(entry.value.clone()).ok()
    }

    pub async fn put<T: Serialize>(&self, key: String, value: &T) {
        let Ok(v) = serde_json::to_value(value) else {
            return;
        };
        let mut map = self.inner.write().await;
        map.insert(
            key,
            CacheEntry {
                value: v,
                inserted_at: Instant::now(),
            },
        );
    }

    pub async fn clear(&self) {
        self.inner.write().await.clear();
    }
}

// ---------- registration ----------

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cache = RegistryCache::new(Duration::from_millis(cfg.registry_cache_ttl_ms));
    register_worker_list(iii, cfg, cache.clone());
    register_worker_info(iii, cfg, cache);
}

fn register_worker_list(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, cache: RegistryCache) {
    let cfg_inner = cfg.clone();
    let cache_inner = cache;
    iii.register_function(
        RegisterFunction::new_async(
            "directory::registry::workers::list",
            move |req: WorkerListInput| {
                let cfg = cfg_inner.clone();
                let cache = cache_inner.clone();
                async move {
                    worker_list(&cfg, &cache, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(
            "List workers from the public registry (api.workers.iii.dev) \
             matching the free-text term `search`. Same row shape as \
             directory::engine::workers::list so callers learn one envelope. \
             Results are cached for `registry_cache_ttl_ms` (default 60s).",
        ),
    );
}

fn register_worker_info(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, cache: RegistryCache) {
    let cfg_inner = cfg.clone();
    let cache_inner = cache;
    iii.register_function(
        RegisterFunction::new_async(
            "directory::registry::workers::info",
            move |req: WorkerInfoInput| {
                let cfg = cfg_inner.clone();
                let cache = cache_inner.clone();
                async move {
                    worker_info(&cfg, &cache, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(
            "Fetch full registry metadata for one worker: worker envelope \
             (same shape as directory::registry::workers::list rows and \
             directory::engine::workers::info), readme, full API reference \
             (functions + triggers schemas), and tree of skill/prompt \
             file paths. Pass either `version` or `tag` (defaults to \
             tag=\"latest\"). Results are cached for `registry_cache_ttl_ms`.",
        ),
    );
}

// ---------- input validation (pure) ----------

/// Resolved version specifier produced by [`classify_worker_info_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerInfoSpec {
    Version(String),
    Tag(String),
}

impl WorkerInfoSpec {
    pub fn as_query_param(&self) -> (&'static str, &str) {
        match self {
            WorkerInfoSpec::Version(v) => ("version", v.as_str()),
            WorkerInfoSpec::Tag(t) => ("tag", t.as_str()),
        }
    }
}

/// Validate the worker-info input shape. Mirrors
/// `crate::functions::download::classify_input` (one of `version` /
/// `tag`, default tag "latest"). Pure so it's unit-testable without
/// the engine or HTTP.
pub fn classify_worker_info_input(
    input: WorkerInfoInput,
) -> Result<(String, WorkerInfoSpec), String> {
    let WorkerInfoInput { name, version, tag } = input;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("name must be non-empty".into());
    }
    let version = version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let tag = tag.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let spec = match (version, tag) {
        (Some(_), Some(_)) => return Err("specify either version OR tag, not both".into()),
        (Some(v), None) => WorkerInfoSpec::Version(v),
        (None, Some(t)) => WorkerInfoSpec::Tag(t),
        (None, None) => WorkerInfoSpec::Tag("latest".into()),
    };
    Ok((name, spec))
}

pub fn clamp_search_limit(limit: Option<u32>) -> u32 {
    let raw = limit.unwrap_or(SEARCH_LIMIT_DEFAULT);
    raw.clamp(1, SEARCH_LIMIT_MAX)
}

// ---------- core handlers ----------

pub async fn worker_list(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    input: WorkerListInput,
) -> Result<WorkerListOutput, String> {
    let q = input.search.as_deref().unwrap_or("").trim().to_string();
    if q.is_empty() {
        return Err("search must be non-empty".into());
    }
    let limit = clamp_search_limit(input.limit);
    let cache_key = format!("worker-list:{q}:{limit}");
    if let Some(cached) = cache.get::<WorkerListOutput>(&cache_key).await {
        return Ok(cached);
    }

    let url = format!("{}/search", cfg.registry_base());
    let client = build_http_client(cfg.download_timeout_ms)?;
    let response = client
        .get(&url)
        .query(&[("q", q.as_str()), ("limit", &limit.to_string())])
        .send()
        .await
        .map_err(|e| format!("GET {url} (q={q}, limit={limit}): {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "registry GET {url} returned HTTP {status}: {}",
            body.trim()
        ));
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|e| format!("decode registry response: {e}"))?;

    let workers = parse_worker_list_response(&body);
    let out = WorkerListOutput { workers };
    cache.put(cache_key, &out).await;
    Ok(out)
}

pub async fn worker_info(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    input: WorkerInfoInput,
) -> Result<WorkerInfoOutput, String> {
    let (name, spec) = classify_worker_info_input(input)?;
    let cache_key = format!(
        "worker-info:{name}:{}={}",
        spec.as_query_param().0,
        spec.as_query_param().1
    );
    if let Some(cached) = cache.get::<WorkerInfoOutput>(&cache_key).await {
        return Ok(cached);
    }

    let url = format!("{}/w/{name}", cfg.registry_base());
    let (key, value) = spec.as_query_param();
    let client = build_http_client(cfg.download_timeout_ms)?;
    let response = client
        .get(&url)
        .query(&[(key, value)])
        .send()
        .await
        .map_err(|e| format!("GET {url} ({key}={value}): {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "registry GET {url} returned HTTP {status}: {}",
            body.trim()
        ));
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|e| format!("decode registry response: {e}"))?;

    let out = parse_worker_info_response(&name, &body);
    cache.put(cache_key, &out).await;
    Ok(out)
}

// ---------- pure response parsers ----------

/// Tolerant parse of the worker-list response. Accepts either the canonical
/// `{ "workers": [...] }` envelope OR a bare array, and silently drops
/// entries that don't include a `name`. Field aliases supported:
/// `latest_version` → `version` (registry uses the longer name when
/// listing, the short one when serving worker-info).
pub fn parse_worker_list_response(value: &Value) -> Vec<Worker> {
    let arr: &[Value] = value
        .get("workers")
        .and_then(|w| w.as_array())
        .or_else(|| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    arr.iter().filter_map(parse_worker_envelope).collect()
}

/// Project a single registry-shaped JSON object into a [`Worker`].
/// Returns `None` if there's no `name` field.
fn parse_worker_envelope(v: &Value) -> Option<Worker> {
    let name = v.get("name").and_then(|n| n.as_str())?.to_string();
    let description = v
        .get("description")
        .and_then(|d| d.as_str())
        .map(String::from);
    let version = v
        .get("version")
        .or_else(|| v.get("latest_version"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let repo = v
        .get("repo")
        .or_else(|| v.get("repository"))
        .and_then(|r| r.as_str())
        .map(String::from);
    let author = v
        .get("author")
        .and_then(|a| serde_json::from_value::<RegistryAuthor>(a.clone()).ok());
    Some(Worker {
        name,
        description,
        version,
        repo,
        author,
    })
}

/// Tolerant parse of the worker-info response. Missing fields default
/// to `None` / empty so a registry that ships partial metadata for a
/// new worker still returns something useful.
pub fn parse_worker_info_response(default_name: &str, value: &Value) -> WorkerInfoOutput {
    // Build the worker envelope, defaulting `name` to `default_name`
    // when the payload omits it.
    let worker = match parse_worker_envelope(value) {
        Some(mut w) if w.name.is_empty() => {
            w.name = default_name.to_string();
            w
        }
        Some(w) => w,
        None => Worker {
            name: default_name.to_string(),
            description: value
                .get("description")
                .and_then(|d| d.as_str())
                .map(String::from),
            version: value
                .get("version")
                .or_else(|| value.get("latest_version"))
                .and_then(|v| v.as_str())
                .map(String::from),
            repo: value
                .get("repo")
                .or_else(|| value.get("repository"))
                .and_then(|r| r.as_str())
                .map(String::from),
            author: value
                .get("author")
                .and_then(|a| serde_json::from_value::<RegistryAuthor>(a.clone()).ok()),
        },
    };

    let readme = value
        .get("readme")
        .and_then(|v| v.as_str())
        .map(String::from);

    // api_reference: accept either a nested object, or a top-level pair
    // of `functions` / `triggers` arrays alongside the rest of the
    // payload (the publish payload uses the flat shape).
    let api_reference = if let Some(api) = value.get("api_reference") {
        serde_json::from_value::<ApiReference>(api.clone()).unwrap_or_default()
    } else {
        let functions = value
            .get("functions")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let triggers = value
            .get("triggers")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let composed = serde_json::json!({
            "functions": functions,
            "triggers": triggers,
        });
        serde_json::from_value::<ApiReference>(composed).unwrap_or_default()
    };

    let skills_tree = value
        .get("skills_tree")
        .or_else(|| value.get("skillsTree"))
        .and_then(|v| serde_json::from_value::<SkillsTree>(v.clone()).ok())
        .unwrap_or_default();

    WorkerInfoOutput {
        worker,
        readme,
        api_reference,
        skills_tree,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_default_uses_latest_tag() {
        let (name, spec) = classify_worker_info_input(WorkerInfoInput {
            name: "resend".into(),
            version: None,
            tag: None,
        })
        .unwrap();
        assert_eq!(name, "resend");
        assert_eq!(spec, WorkerInfoSpec::Tag("latest".into()));
    }

    #[test]
    fn classify_picks_version_over_tag_default() {
        let (_, spec) = classify_worker_info_input(WorkerInfoInput {
            name: "resend".into(),
            version: Some("1.2.3".into()),
            tag: None,
        })
        .unwrap();
        assert_eq!(spec, WorkerInfoSpec::Version("1.2.3".into()));
    }

    #[test]
    fn classify_rejects_both_version_and_tag() {
        let err = classify_worker_info_input(WorkerInfoInput {
            name: "resend".into(),
            version: Some("1.2.3".into()),
            tag: Some("latest".into()),
        })
        .unwrap_err();
        assert!(err.contains("either version OR tag"), "got: {err}");
    }

    #[test]
    fn classify_rejects_empty_name() {
        let err = classify_worker_info_input(WorkerInfoInput {
            name: "  ".into(),
            version: None,
            tag: None,
        })
        .unwrap_err();
        assert!(err.contains("name"), "got: {err}");
    }

    #[test]
    fn classify_trims_whitespace() {
        let (name, spec) = classify_worker_info_input(WorkerInfoInput {
            name: "  agent-memory  ".into(),
            version: None,
            tag: Some("  latest\n".into()),
        })
        .unwrap();
        assert_eq!(name, "agent-memory");
        assert_eq!(spec, WorkerInfoSpec::Tag("latest".into()));
    }

    #[test]
    fn clamp_search_limit_caps_to_max() {
        assert_eq!(clamp_search_limit(None), SEARCH_LIMIT_DEFAULT);
        assert_eq!(clamp_search_limit(Some(10)), 10);
        assert_eq!(clamp_search_limit(Some(500)), SEARCH_LIMIT_MAX);
        assert_eq!(clamp_search_limit(Some(0)), 1);
    }

    #[test]
    fn parse_worker_list_response_accepts_envelope() {
        let v = json!({
            "workers": [
                {
                    "name": "resend",
                    "latest_version": "1.2.3",
                    "description": "Email worker",
                    "repo": "https://github.com/iii/resend",
                    "author": { "name": "iii", "is_verified": true }
                }
            ]
        });
        let workers = parse_worker_list_response(&v);
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "resend");
        assert_eq!(workers[0].version.as_deref(), Some("1.2.3"));
        assert!(workers[0].author.as_ref().unwrap().is_verified);
    }

    #[test]
    fn parse_worker_list_response_accepts_bare_array() {
        let v = json!([
            { "name": "alpha" },
            { "name": "beta", "description": "desc" }
        ]);
        let workers = parse_worker_list_response(&v);
        assert_eq!(workers.len(), 2);
        assert_eq!(workers[1].description.as_deref(), Some("desc"));
    }

    #[test]
    fn parse_worker_list_response_drops_invalid_entries() {
        let v = json!({ "workers": [ { "no_name": true }, { "name": "ok" } ] });
        let workers = parse_worker_list_response(&v);
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "ok");
    }

    #[test]
    fn parse_worker_info_handles_flat_publish_payload() {
        let v = json!({
            "name": "resend",
            "version": "1.2.3",
            "description": "Email worker",
            "repo": "https://github.com/iii/resend",
            "readme": "# Resend\n\nDocs here.",
            "author": { "name": "iii", "is_verified": true },
            "functions": [
                {
                    "name": "send",
                    "description": "Send an email.",
                    "request_schema": { "type": "object" },
                    "response_schema": { "type": "object" }
                }
            ],
            "triggers": [
                {
                    "name": "on-bounce",
                    "description": "Fires when a delivery bounces.",
                    "invocation_schema": { "type": "object" }
                }
            ],
            "skills_tree": {
                "skills": [ { "path": "index.md" } ],
                "prompts": [ { "name": "send-email", "description": "Compose." } ]
            }
        });
        let out = parse_worker_info_response("resend", &v);
        assert_eq!(out.worker.name, "resend");
        assert_eq!(out.worker.version.as_deref(), Some("1.2.3"));
        assert_eq!(out.worker.description.as_deref(), Some("Email worker"));
        assert!(out.readme.as_deref().unwrap().contains("# Resend"));
        assert_eq!(out.api_reference.functions.len(), 1);
        assert_eq!(out.api_reference.functions[0].name, "send");
        assert_eq!(out.api_reference.triggers.len(), 1);
        assert_eq!(out.skills_tree.skills.len(), 1);
        assert_eq!(out.skills_tree.prompts.len(), 1);
        assert!(out.worker.author.as_ref().unwrap().is_verified);
    }

    #[test]
    fn parse_worker_info_handles_nested_api_reference() {
        let v = json!({
            "name": "x",
            "api_reference": {
                "functions": [{ "name": "f1" }],
                "triggers": []
            }
        });
        let out = parse_worker_info_response("x", &v);
        assert_eq!(out.api_reference.functions.len(), 1);
        assert!(out.api_reference.triggers.is_empty());
    }

    #[test]
    fn parse_worker_info_falls_back_to_default_name() {
        let v = json!({});
        let out = parse_worker_info_response("fallback", &v);
        assert_eq!(out.worker.name, "fallback");
        assert!(out.worker.version.is_none());
        assert!(out.api_reference.functions.is_empty());
    }

    #[tokio::test]
    async fn registry_cache_returns_within_ttl() {
        let cache = RegistryCache::new(Duration::from_millis(500));
        cache.put("k".into(), &json!({"x": 1})).await;
        let v: serde_json::Value = cache.get("k").await.unwrap();
        assert_eq!(v["x"], 1);
    }

    #[tokio::test]
    async fn registry_cache_expires_after_ttl() {
        let cache = RegistryCache::new(Duration::from_millis(20));
        cache.put("k".into(), &json!({"x": 1})).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let v: Option<serde_json::Value> = cache.get("k").await;
        assert!(v.is_none());
    }

    #[tokio::test]
    async fn registry_cache_clear_removes_all() {
        let cache = RegistryCache::new(Duration::from_millis(500));
        cache.put("k".into(), &json!({"x": 1})).await;
        cache.clear().await;
        let v: Option<serde_json::Value> = cache.get("k").await;
        assert!(v.is_none());
    }
}
