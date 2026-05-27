//! `directory::registry::*` — HTTP proxy over
//! `https://api.workers.iii.dev`.
//!
//! Two functions, mirroring `directory::engine::workers::*` so callers
//! learn one shape:
//!
//!   * `directory::registry::workers::list`  — list workers in the
//!     public registry, filterable by `search` and paginated via
//!     opaque `cursor`. Same row envelope (`Worker`) as
//!     [`crate::functions::directory::Worker`] for the shared core
//!     fields (`name`, `description`, `version`).
//!   * `directory::registry::workers::info`  — full registry metadata
//!     for one worker. Wraps the registry-side fields in a top-level
//!     `worker` envelope (same shape as the list rows), with `readme`
//!     / `api_reference` / `skills_tree` as surface-specific extras.
//!     `skills_tree` is fetched from a separate `/w/{slug}/skills`
//!     endpoint in parallel and merged into the response.
//!
//! Both responses are cached in-process for `registry_cache_ttl_ms`
//! (default 60s) so repeat lookups don't hammer the registry — every
//! call is a separate HTTP request without it.
//!
//! Wire contract (mirrors `openapi.yaml`):
//!
//!   * `GET {base}/w?search=…&cursor=…` →
//!     `{ workers: [WorkerListItem], pagination: { next_cursor, has_more, page_size } }`.
//!     Page size is server-authored; the client cannot override it.
//!   * `GET {base}/w/{slug}?version=…` → `{ worker: WorkerDetail }`.
//!     `version` accepts either a tag (`latest`) or an exact semver
//!     (`1.2.3`). The legacy `?tag=` query param is not used.
//!   * `GET {base}/w/{slug}/skills?version=…` →
//!     `{ name, version, skills: [{path, content}], prompts: [{name, description?, args_schema?, content}] }`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::config::SkillsConfig;
use crate::sources::build_http_client;

// ---------- public input/output shapes ----------

/// `directory::registry::workers::list` input. Mirrors
/// [`crate::functions::directory::WorkerListInput.search`] so callers
/// can switch between local and registry surfaces without re-learning
/// the API. Adds `cursor` for paging because the registry is paged
/// (server-authored page size — the client cannot override it).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkerListInput {
    /// Optional free-text query. Forwarded to the registry as
    /// `?search=…`; the registry ranks results by `pg_trgm` similarity
    /// against `lower(name)` and `lower(description)`. When omitted,
    /// results are ordered by `total_downloads DESC`.
    #[serde(default)]
    pub search: Option<String>,
    /// Opaque cursor returned by a previous call's
    /// `pagination.next_cursor`. Pass back verbatim to fetch the next
    /// page; omit (or pass `null`) to fetch the first page.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Author block for a published worker. Field names match the
/// `WorkerAuthor` schema in `openapi.yaml` (`pfp`, `verified`).
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct WorkerAuthor {
    pub name: Option<String>,
    /// Profile picture URL. `null` when the author hasn't uploaded one.
    #[serde(default)]
    pub pfp: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

/// Worker dependency entry. Mirrors the `Dependency` schema in
/// `openapi.yaml`.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

/// Shared worker envelope used by both
/// `directory::registry::workers::list` rows and the `worker` field of
/// `directory::registry::workers::info`. Field names match the
/// OpenAPI `WorkerListItem` schema. The shared core fields (`name`,
/// `description`, `version`) line up with
/// [`crate::functions::directory::Worker`] so callers learn one shape
/// across local + registry surfaces.
#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct Worker {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Worker kind — `binary`, `image`, or `engine`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Latest published version (worker-list) or the resolved version
    /// (worker-info, when called with `version` / `tag`).
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    /// Free-form runtime configuration block from the publish payload.
    #[serde(default = "default_config_object")]
    pub config: Value,
    #[serde(default)]
    pub supported_targets: Vec<String>,
    /// Container image tag, populated only for `type=image` workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub total_downloads: u64,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub author: Option<WorkerAuthor>,
}

fn default_config_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Pagination envelope returned alongside a worker-list page. Mirrors
/// the OpenAPI `Pagination` schema.
#[derive(Debug, Serialize, Deserialize, Clone, Default, JsonSchema)]
pub struct Pagination {
    /// Opaque cursor for the next page. `null` on the last page.
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    /// Server-authored page size. The client cannot override this.
    #[serde(default)]
    pub page_size: u32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkerListOutput {
    pub workers: Vec<Worker>,
    pub pagination: Pagination,
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
        "directory::registry::workers::list",
        RegisterFunction::new_async(move |req: WorkerListInput| {
            let cfg = cfg_inner.clone();
            let cache = cache_inner.clone();
            async move {
                worker_list(&cfg, &cache, req)
                    .await
                    .map_err(IIIError::Handler)
            }
        })
        .description(
            "List workers from the public registry (api.workers.iii.dev). \
             Optional free-text `search` is matched fuzzily by the registry; \
             omit it to browse by `total_downloads DESC`. Pagination is \
             cursor-based with a server-authored page size — pass back \
             `pagination.next_cursor` as `cursor` to fetch the next page. \
             Shares the core `name` / `description` / `version` fields with \
             directory::engine::workers::list. Results are cached for \
             `registry_cache_ttl_ms` (default 60s).",
        ),
    );
}

fn register_worker_info(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, cache: RegistryCache) {
    let cfg_inner = cfg.clone();
    let cache_inner = cache;
    iii.register_function(
        "directory::registry::workers::info",
        RegisterFunction::new_async(move |req: WorkerInfoInput| {
            let cfg = cfg_inner.clone();
            let cache = cache_inner.clone();
            async move {
                worker_info(&cfg, &cache, req)
                    .await
                    .map_err(IIIError::Handler)
            }
        })
        .description(
            "Fetch full registry metadata for one worker: worker envelope \
             (same core fields as directory::engine::workers::info plus \
             registry-only `type` / `config` / `supported_targets` / \
             `total_downloads` / `dependencies` / `image`), readme, full \
             API reference (functions + triggers schemas), and the tree \
             of skill / prompt file paths fetched from the registry's \
             /w/{slug}/skills endpoint. Pass either `version` or `tag` \
             (defaults to tag=\"latest\"). Results are cached for \
             `registry_cache_ttl_ms`.",
        ),
    );
}

// ---------- input validation (pure) ----------

/// Resolved version specifier produced by [`classify_worker_info_input`].
/// Both arms serialise as `?version=…` on the wire (the OpenAPI
/// `/w/{slug}` and `/w/{slug}/skills` endpoints accept either a tag
/// or an exact semver under the same `version` query param). The enum
/// distinction is preserved purely so the cache key, error messages,
/// and unit tests can tell which user-facing input was supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerInfoSpec {
    Version(String),
    Tag(String),
}

impl WorkerInfoSpec {
    /// User-facing label (for cache keys and diagnostics). The wire
    /// query key is always `version` — see [`Self::query_value`].
    pub fn label(&self) -> &'static str {
        match self {
            WorkerInfoSpec::Version(_) => "version",
            WorkerInfoSpec::Tag(_) => "tag",
        }
    }

    /// The `?version=…` value to send to the registry. Tags and
    /// semvers go down the same wire path per the OpenAPI contract.
    pub fn query_value(&self) -> &str {
        match self {
            WorkerInfoSpec::Version(v) => v.as_str(),
            WorkerInfoSpec::Tag(t) => t.as_str(),
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

// ---------- core handlers ----------

pub async fn worker_list(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    input: WorkerListInput,
) -> Result<WorkerListOutput, String> {
    let search = input
        .search
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let cursor = input
        .cursor
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let cache_key = format!(
        "worker-list:{}:{}",
        search.as_deref().unwrap_or(""),
        cursor.as_deref().unwrap_or("")
    );
    if let Some(cached) = cache.get::<WorkerListOutput>(&cache_key).await {
        return Ok(cached);
    }

    let url = format!("{}/w", cfg.registry_base());
    let client = build_http_client(cfg.download_timeout_ms)?;
    let mut request = client.get(&url);
    let mut query: Vec<(&str, &str)> = Vec::with_capacity(2);
    if let Some(s) = search.as_deref() {
        query.push(("search", s));
    }
    if let Some(c) = cursor.as_deref() {
        query.push(("cursor", c));
    }
    if !query.is_empty() {
        request = request.query(&query);
    }

    let response = request.send().await.map_err(|e| {
        format!(
            "GET {url} (search={:?}, cursor={:?}): {e}",
            search.as_deref().unwrap_or(""),
            cursor.as_deref().unwrap_or("")
        )
    })?;
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

    let out = parse_worker_list_response(&body);
    cache.put(cache_key, &out).await;
    Ok(out)
}

pub async fn worker_info(
    cfg: &SkillsConfig,
    cache: &RegistryCache,
    input: WorkerInfoInput,
) -> Result<WorkerInfoOutput, String> {
    let (name, spec) = classify_worker_info_input(input)?;
    let cache_key = format!("worker-info:{name}:{}={}", spec.label(), spec.query_value());
    if let Some(cached) = cache.get::<WorkerInfoOutput>(&cache_key).await {
        return Ok(cached);
    }

    let base = cfg.registry_base();
    let detail_url = format!("{base}/w/{name}");
    let skills_url = format!("{base}/w/{name}/skills");
    let version_value = spec.query_value().to_string();
    let client = build_http_client(cfg.download_timeout_ms)?;

    // Fan out the two GETs concurrently. Per OpenAPI both endpoints
    // accept `?version=` for either tag or exact semver, so both legs
    // share the same query value.
    let detail_fut = fetch_json(&client, &detail_url, &version_value);
    let skills_fut = fetch_json(&client, &skills_url, &version_value);
    let (detail_body, skills_body) = tokio::try_join!(detail_fut, skills_fut)?;

    let out = parse_worker_info_response(&name, &detail_body, &skills_body);
    cache.put(cache_key, &out).await;
    Ok(out)
}

/// Issue `GET {url}?version={version}` and decode the body as JSON.
/// Surfaces non-2xx statuses as `Err(String)` so the caller can fail
/// the whole `worker_info` call.
async fn fetch_json(
    client: &reqwest::Client,
    url: &str,
    version_value: &str,
) -> Result<Value, String> {
    let response = client
        .get(url)
        .query(&[("version", version_value)])
        .send()
        .await
        .map_err(|e| format!("GET {url} (version={version_value}): {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "registry GET {url} returned HTTP {status}: {}",
            body.trim()
        ));
    }
    response
        .json::<Value>()
        .await
        .map_err(|e| format!("decode registry response from {url}: {e}"))
}

// ---------- pure response parsers ----------

/// Parse the `WorkerListResponse` envelope returned by `GET /w`.
/// Tolerates a missing `pagination` block (defaults to "no more pages")
/// so a future server hotfix can't break the worker. Entries without
/// a `name` field are dropped silently.
pub fn parse_worker_list_response(value: &Value) -> WorkerListOutput {
    let workers = value
        .get("workers")
        .and_then(|w| w.as_array())
        .map(|arr| arr.iter().filter_map(parse_worker_envelope).collect())
        .unwrap_or_default();
    let pagination = value
        .get("pagination")
        .and_then(|p| serde_json::from_value::<Pagination>(p.clone()).ok())
        .unwrap_or_default();
    WorkerListOutput {
        workers,
        pagination,
    }
}

/// Project a single registry-shaped `WorkerListItem` JSON object into
/// a [`Worker`]. Returns `None` if there's no `name` field.
fn parse_worker_envelope(v: &Value) -> Option<Worker> {
    v.get("name").and_then(|n| n.as_str())?;
    serde_json::from_value::<Worker>(v.clone()).ok()
}

/// Parse the `{ worker: WorkerDetail }` envelope from `GET /w/{slug}`
/// merged with the `WorkerSkillsDownloadResponse` from
/// `GET /w/{slug}/skills`. Both inputs are tolerated when partially
/// populated so a registry that ships partial metadata for a new
/// worker still returns something useful.
pub fn parse_worker_info_response(
    default_name: &str,
    detail_body: &Value,
    skills_body: &Value,
) -> WorkerInfoOutput {
    // The OpenAPI `/w/{slug}` response is `{ worker: WorkerDetail }`.
    // Fall back to the top-level object for forward-compatibility with
    // any older deployment that returned a flat publish payload.
    let detail = detail_body.get("worker").unwrap_or(detail_body);

    let worker = parse_worker_envelope(detail).unwrap_or_else(|| Worker {
        name: default_name.to_string(),
        description: None,
        kind: None,
        version: None,
        repo: None,
        config: default_config_object(),
        supported_targets: Vec::new(),
        image: None,
        total_downloads: 0,
        dependencies: Vec::new(),
        author: None,
    });

    let readme = detail
        .get("readme")
        .and_then(|v| v.as_str())
        .map(String::from);

    let functions = detail
        .get("functions")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let triggers = detail
        .get("triggers")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let api_reference = serde_json::from_value::<ApiReference>(serde_json::json!({
        "functions": functions,
        "triggers": triggers,
    }))
    .unwrap_or_default();

    let skills_tree = parse_skills_tree(skills_body);

    WorkerInfoOutput {
        worker,
        readme,
        api_reference,
        skills_tree,
    }
}

/// Project a `WorkerSkillsDownloadResponse` body into the metadata-only
/// `SkillsTree` shape. Drops markdown bodies (`content`) and prompt
/// `args_schema` / `content` since this view is for picker UIs, not
/// content rendering. Use `directory::skills::download` to materialise
/// the bodies on disk.
fn parse_skills_tree(value: &Value) -> SkillsTree {
    let skills = value
        .get("skills")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    s.get("path")
                        .and_then(|p| p.as_str())
                        .map(|p| SkillsTreeSkill {
                            path: p.to_string(),
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    let prompts = value
        .get("prompts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let name = p.get("name").and_then(|n| n.as_str())?.to_string();
                    let description = p
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(String::from);
                    Some(SkillsTreePrompt { name, description })
                })
                .collect()
        })
        .unwrap_or_default();
    SkillsTree { skills, prompts }
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
    fn worker_info_spec_label_distinguishes_arms() {
        assert_eq!(WorkerInfoSpec::Version("1.2.3".into()).label(), "version");
        assert_eq!(WorkerInfoSpec::Tag("latest".into()).label(), "tag");
    }

    #[test]
    fn worker_info_spec_query_value_uses_inner_string() {
        assert_eq!(
            WorkerInfoSpec::Version("1.2.3".into()).query_value(),
            "1.2.3"
        );
        assert_eq!(WorkerInfoSpec::Tag("latest".into()).query_value(), "latest");
    }

    #[test]
    fn parse_worker_list_response_accepts_envelope_with_pagination() {
        let v = json!({
            "workers": [
                {
                    "name": "resend",
                    "description": "Email worker",
                    "type": "binary",
                    "version": "1.2.3",
                    "repo": "https://github.com/iii/resend",
                    "config": { "runtime": "edge" },
                    "supported_targets": ["x86_64-unknown-linux-gnu"],
                    "total_downloads": 42,
                    "dependencies": [],
                    "author": { "name": "iii", "pfp": null, "verified": true }
                }
            ],
            "pagination": {
                "next_cursor": "opaque-cursor",
                "has_more": true,
                "page_size": 20
            }
        });
        let out = parse_worker_list_response(&v);
        assert_eq!(out.workers.len(), 1);
        assert_eq!(out.workers[0].name, "resend");
        assert_eq!(out.workers[0].kind.as_deref(), Some("binary"));
        assert_eq!(out.workers[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(out.workers[0].total_downloads, 42);
        assert_eq!(
            out.workers[0].supported_targets,
            vec!["x86_64-unknown-linux-gnu".to_string()]
        );
        let author = out.workers[0].author.as_ref().unwrap();
        assert_eq!(author.name.as_deref(), Some("iii"));
        assert!(author.verified);
        assert!(author.pfp.is_none());
        assert_eq!(out.pagination.next_cursor.as_deref(), Some("opaque-cursor"));
        assert!(out.pagination.has_more);
        assert_eq!(out.pagination.page_size, 20);
    }

    #[test]
    fn parse_worker_list_response_defaults_pagination_when_missing() {
        let v = json!({ "workers": [ { "name": "alpha" } ] });
        let out = parse_worker_list_response(&v);
        assert_eq!(out.workers.len(), 1);
        assert_eq!(out.workers[0].name, "alpha");
        assert!(out.pagination.next_cursor.is_none());
        assert!(!out.pagination.has_more);
        assert_eq!(out.pagination.page_size, 0);
    }

    #[test]
    fn parse_worker_list_response_drops_entries_without_name() {
        let v = json!({ "workers": [ { "no_name": true }, { "name": "ok" } ] });
        let out = parse_worker_list_response(&v);
        assert_eq!(out.workers.len(), 1);
        assert_eq!(out.workers[0].name, "ok");
    }

    #[test]
    fn parse_worker_list_response_handles_empty_envelope() {
        let v = json!({});
        let out = parse_worker_list_response(&v);
        assert!(out.workers.is_empty());
        assert!(!out.pagination.has_more);
    }

    #[test]
    fn parse_worker_info_unwraps_worker_envelope_and_merges_skills() {
        let detail = json!({
            "worker": {
                "name": "resend",
                "description": "Email worker",
                "type": "binary",
                "version": "1.2.3",
                "repo": "https://github.com/iii/resend",
                "config": {},
                "supported_targets": ["x86_64-unknown-linux-gnu"],
                "total_downloads": 1234,
                "dependencies": [{ "name": "todo-worker", "version": "0.1.0" }],
                "author": { "name": "iii", "pfp": null, "verified": true },
                "readme": "# Resend\n\nDocs here.",
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
                        "invocation_schema": { "type": "object" },
                        "return_schema": { "type": "object" }
                    }
                ]
            }
        });
        let skills = json!({
            "name": "resend",
            "version": "1.2.3",
            "skills": [
                { "path": "index.md", "content": "ignored" },
                { "path": "emails/send-email.md", "content": "ignored" }
            ],
            "prompts": [
                {
                    "name": "send-email",
                    "description": "Compose.",
                    "args_schema": { "type": "object" },
                    "content": "ignored"
                }
            ]
        });
        let out = parse_worker_info_response("resend", &detail, &skills);
        assert_eq!(out.worker.name, "resend");
        assert_eq!(out.worker.version.as_deref(), Some("1.2.3"));
        assert_eq!(out.worker.description.as_deref(), Some("Email worker"));
        assert_eq!(out.worker.kind.as_deref(), Some("binary"));
        assert_eq!(out.worker.total_downloads, 1234);
        assert_eq!(out.worker.dependencies.len(), 1);
        assert_eq!(out.worker.dependencies[0].name, "todo-worker");
        assert!(out.worker.author.as_ref().unwrap().verified);
        assert!(out.readme.as_deref().unwrap().contains("# Resend"));
        assert_eq!(out.api_reference.functions.len(), 1);
        assert_eq!(out.api_reference.functions[0].name, "send");
        assert_eq!(out.api_reference.triggers.len(), 1);
        assert_eq!(out.api_reference.triggers[0].name, "on-bounce");
        assert_eq!(out.skills_tree.skills.len(), 2);
        assert_eq!(out.skills_tree.skills[0].path, "index.md");
        assert_eq!(out.skills_tree.prompts.len(), 1);
        assert_eq!(out.skills_tree.prompts[0].name, "send-email");
        assert_eq!(
            out.skills_tree.prompts[0].description.as_deref(),
            Some("Compose.")
        );
    }

    #[test]
    fn parse_worker_info_falls_back_to_default_name_on_empty_bodies() {
        let detail = json!({});
        let skills = json!({ "skills": [], "prompts": [] });
        let out = parse_worker_info_response("fallback", &detail, &skills);
        assert_eq!(out.worker.name, "fallback");
        assert!(out.worker.version.is_none());
        assert!(out.api_reference.functions.is_empty());
        assert!(out.skills_tree.skills.is_empty());
        assert!(out.skills_tree.prompts.is_empty());
    }

    #[test]
    fn parse_worker_info_tolerates_legacy_flat_payload() {
        // Forward-compat: an older deployment that returns the flat
        // publish payload (no `worker` key) is still readable.
        let detail = json!({
            "name": "legacy",
            "version": "0.1.0",
            "description": "Legacy worker",
            "type": "binary",
            "config": {},
            "supported_targets": [],
            "total_downloads": 0,
            "dependencies": [],
            "author": null,
            "functions": [{ "name": "f1" }],
            "triggers": []
        });
        let skills = json!({ "skills": [], "prompts": [] });
        let out = parse_worker_info_response("legacy", &detail, &skills);
        assert_eq!(out.worker.name, "legacy");
        assert_eq!(out.api_reference.functions.len(), 1);
    }

    #[test]
    fn parse_skills_tree_drops_entries_without_path_or_name() {
        let v = json!({
            "skills": [
                { "path": "ok.md", "content": "x" },
                { "content": "missing path" }
            ],
            "prompts": [
                { "name": "ok", "description": "fine" },
                { "description": "missing name" }
            ]
        });
        let tree = parse_skills_tree(&v);
        assert_eq!(tree.skills.len(), 1);
        assert_eq!(tree.skills[0].path, "ok.md");
        assert_eq!(tree.prompts.len(), 1);
        assert_eq!(tree.prompts[0].name, "ok");
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
