//! Client for the registry's kit endpoints (`/k/*`, `/blobs/*`), see
//! `registry/docs/KITS_API.md`. Shapes are read leniently (`#[serde(default)]`
//! everywhere): a field the registry stops sending degrades a preview, never
//! an install.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sources::build_http_client;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Author {
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Deprecation {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub replaced_by: Option<String>,
}

/// `AgentEntry`: parsed frontmatter of `agents/<id>.md`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkillEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitWorker {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub range: String,
    #[serde(default)]
    pub resolved: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitCounts {
    #[serde(default)]
    pub agents: u64,
    #[serde(default)]
    pub skills: u64,
    #[serde(default)]
    pub workers: u64,
}

/// `KitDetail` (`GET /k/:author/:name?version=`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub release_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub counts: KitCounts,
    #[serde(default)]
    pub deprecation: Option<Deprecation>,
    #[serde(default)]
    pub experimental: Option<bool>,
    #[serde(default)]
    pub workers: Vec<KitWorker>,
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
    #[serde(default)]
    pub skills: Vec<SkillEntry>,
    #[serde(default)]
    pub total_downloads: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitVersion {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitVersions {
    #[serde(default)]
    pub release_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub versions: Vec<KitVersion>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitFile {
    pub path: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub content: String,
}

/// `GET /k/:author/:name/files` and `/download`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitFiles {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub files: Vec<KitFile>,
    #[serde(default)]
    pub workers: Vec<KitWorker>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitFunction {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub used_by: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub worker: Option<String>,
    #[serde(default)]
    pub worker_version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// One entry of `compare.agents.modified`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComparedAgent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub frontmatter: Value,
    #[serde(default)]
    pub body_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComparedAgents {
    #[serde(default)]
    pub modified: Vec<ComparedAgent>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkerRange {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ComparedWorkers {
    #[serde(default)]
    pub added: Vec<WorkerRange>,
    #[serde(default)]
    pub removed: Vec<WorkerRange>,
    #[serde(default)]
    pub changed: Vec<WorkerRange>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionAdded {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub function: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelChanged {
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    #[serde(default)]
    pub workers_added: Vec<String>,
    #[serde(default)]
    pub functions_added: Vec<FunctionAdded>,
    #[serde(default)]
    pub models_changed: Vec<ModelChanged>,
}

/// `GET /k/:author/:name/compare?from=&to=`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitCompare {
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub major: Option<bool>,
    #[serde(default)]
    pub agents: ComparedAgents,
    #[serde(default)]
    pub workers: ComparedWorkers,
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateQuery {
    pub kit: String,
    pub version: String,
    pub requested: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateAnswer {
    #[serde(default)]
    pub kit: String,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub available: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub major: bool,
    #[serde(default)]
    pub deprecation: Option<Deprecation>,
    #[serde(default)]
    pub not_found: bool,
}

/// What the kit flows need from the registry. The HTTP implementation is
/// [`HttpKitRegistry`]; tests substitute an in-memory one.
#[async_trait]
pub trait KitRegistry: Send + Sync {
    /// `None` when the kit (or that version) does not exist.
    async fn detail(&self, kit: &str, version: &str) -> Result<Option<KitDetail>, String>;
    async fn versions(&self, kit: &str) -> Result<Option<KitVersions>, String>;
    /// File contents of a version, NOT counted as a download.
    async fn files(&self, kit: &str, version: &str) -> Result<KitFiles, String>;
    /// The counted download (`ci` counts it as CI).
    async fn download(&self, kit: &str, version: &str, ci: bool) -> Result<KitFiles, String>;
    async fn compare(&self, kit: &str, from: &str, to: &str) -> Result<Option<KitCompare>, String>;
    async fn functions(&self, kit: &str, version: &str) -> Result<Vec<KitFunction>, String>;
    async fn readme(&self, kit: &str, version: &str) -> Result<Option<String>, String>;
    async fn updates(&self, queries: &[UpdateQuery]) -> Result<Vec<UpdateAnswer>, String>;
    /// Raw file body by content hash; `None` when unknown.
    async fn blob(&self, sha256: &str) -> Result<Option<String>, String>;
}

/// The registry over HTTP. Blobs are immutable, so they are memoized.
pub struct HttpKitRegistry {
    base: String,
    timeout_ms: u64,
    blobs: Mutex<HashMap<String, String>>,
}

impl HttpKitRegistry {
    pub fn new(base: &str, timeout_ms: u64) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            timeout_ms,
            blobs: Mutex::new(HashMap::new()),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<reqwest::Response>, String> {
        let client = build_http_client(self.timeout_ms)?;
        let response = client
            .get(self.url(path))
            .query(query)
            .send()
            .await
            .map_err(|_| unreachable_registry())?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(registry_status(response.status().as_u16()));
        }
        Ok(Some(response))
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Option<T>, String> {
        match self.get(path, query).await? {
            Some(response) => response.json::<T>().await.map(Some).map_err(|e| {
                format!(
                    "D520 registry_error: could not decode the registry response for {path}: {e}"
                )
            }),
            None => Ok(None),
        }
    }
}

fn unreachable_registry() -> String {
    "D520 registry_error: could not reach the registry. Next: retry shortly.".to_string()
}

fn registry_status(status: u16) -> String {
    format!("D520 registry_error: registry returned HTTP {status}. Next: retry shortly.")
}

fn kit_path(kit: &str) -> String {
    format!("/k/{kit}")
}

#[derive(Deserialize)]
struct DetailEnvelope {
    kit: KitDetail,
}

#[derive(Deserialize)]
struct FunctionsEnvelope {
    #[serde(default)]
    functions: Vec<KitFunction>,
}

#[derive(Deserialize)]
struct UpdatesEnvelope {
    #[serde(default)]
    kits: Vec<UpdateAnswer>,
}

#[async_trait]
impl KitRegistry for HttpKitRegistry {
    async fn detail(&self, kit: &str, version: &str) -> Result<Option<KitDetail>, String> {
        Ok(self
            .get_json::<DetailEnvelope>(&kit_path(kit), &[("version", version)])
            .await?
            .map(|e| e.kit))
    }

    async fn versions(&self, kit: &str) -> Result<Option<KitVersions>, String> {
        self.get_json(&format!("{}/versions", kit_path(kit)), &[])
            .await
    }

    async fn files(&self, kit: &str, version: &str) -> Result<KitFiles, String> {
        self.get_json(&format!("{}/files", kit_path(kit)), &[("version", version)])
            .await?
            .ok_or_else(|| format!("D510 not_found: kit {kit:?} has no version {version:?}."))
    }

    async fn download(&self, kit: &str, version: &str, ci: bool) -> Result<KitFiles, String> {
        let ci = if ci { "true" } else { "false" };
        self.get_json(
            &format!("{}/download", kit_path(kit)),
            &[("version", version), ("ci", ci)],
        )
        .await?
        .ok_or_else(|| format!("D510 not_found: kit {kit:?} has no version {version:?}."))
    }

    async fn compare(&self, kit: &str, from: &str, to: &str) -> Result<Option<KitCompare>, String> {
        self.get_json(
            &format!("{}/compare", kit_path(kit)),
            &[("from", from), ("to", to)],
        )
        .await
    }

    async fn functions(&self, kit: &str, version: &str) -> Result<Vec<KitFunction>, String> {
        Ok(self
            .get_json::<FunctionsEnvelope>(
                &format!("{}/functions", kit_path(kit)),
                &[("version", version)],
            )
            .await?
            .map(|e| e.functions)
            .unwrap_or_default())
    }

    async fn readme(&self, kit: &str, version: &str) -> Result<Option<String>, String> {
        match self
            .get(
                &format!("{}/readme", kit_path(kit)),
                &[("version", version)],
            )
            .await?
        {
            Some(response) => response
                .text()
                .await
                .map(Some)
                .map_err(|_| "D520 registry_error: could not read the kit README.".to_string()),
            None => Ok(None),
        }
    }

    async fn updates(&self, queries: &[UpdateQuery]) -> Result<Vec<UpdateAnswer>, String> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let client = build_http_client(self.timeout_ms)?;
        let response = client
            .post(self.url("/k/updates"))
            .json(&serde_json::json!({ "kits": queries }))
            .send()
            .await
            .map_err(|_| unreachable_registry())?;
        if !response.status().is_success() {
            return Err(registry_status(response.status().as_u16()));
        }
        let body: UpdatesEnvelope = response
            .json()
            .await
            .map_err(|e| format!("D520 registry_error: could not decode /k/updates: {e}"))?;
        Ok(body.kits)
    }

    async fn blob(&self, sha256: &str) -> Result<Option<String>, String> {
        if let Some(hit) = self
            .blobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(sha256)
            .cloned()
        {
            return Ok(Some(hit));
        }
        if !sha256.chars().all(|c| c.is_ascii_hexdigit()) || sha256.len() != 64 {
            return Ok(None);
        }
        let Some(response) = self.get(&format!("/blobs/{sha256}"), &[]).await? else {
            return Ok(None);
        };
        let body = response
            .text()
            .await
            .map_err(|_| "D520 registry_error: could not read a blob.".to_string())?;
        self.blobs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(sha256.to_string(), body.clone());
        Ok(Some(body))
    }
}

/// Web page of a kit, derived from the API base: `api.<host>` → `<host>`.
/// `registry_web_url` (config) overrides the derivation.
pub fn kit_web_url(api_base: &str, web_override: Option<&str>, kit: &str) -> String {
    let base = match web_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(web) => web.trim_end_matches('/').to_string(),
        None => api_base.trim_end_matches('/').replacen("://api.", "://", 1),
    };
    format!("{base}/kits/{kit}")
}

/// Web page of a registry worker (see [`kit_web_url`]).
pub fn worker_web_url(api_base: &str, web_override: Option<&str>, worker: &str) -> String {
    let base = match web_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(web) => web.trim_end_matches('/').to_string(),
        None => api_base.trim_end_matches('/').replacen("://api.", "://", 1),
    };
    format!("{base}/workers/{worker}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_urls_strip_the_api_host_unless_overridden() {
        assert_eq!(
            kit_web_url("https://api.registry.example", None, "acme/k"),
            "https://registry.example/kits/acme/k"
        );
        assert_eq!(
            kit_web_url(
                "http://localhost:4912",
                Some("http://localhost:4914/"),
                "acme/k"
            ),
            "http://localhost:4914/kits/acme/k"
        );
        assert_eq!(
            worker_web_url("https://api.workers.iii.dev", None, "kanban"),
            "https://workers.iii.dev/workers/kanban"
        );
    }

    #[tokio::test]
    async fn http_registry_reads_detail_and_maps_404_to_none() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/k/acme/kanban-team"))
            .and(query_param("version", "latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit": {
                    "id": "acme/kanban-team", "name": "kanban-team", "version": "1.3.0",
                    "author": { "handle": "acme", "name": "Acme", "verified": true },
                    "agents": [{ "id": "planner", "path": "agents/planner.md", "sha256": "aa", "model": "opus" }],
                    "workers": [{ "name": "kanban", "range": "^1.4", "resolved": "1.6.1" }],
                    "unknown_field": 1
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/k/acme/missing"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({"error": "Kit not found"})),
            )
            .mount(&server)
            .await;

        let reg = HttpKitRegistry::new(&server.uri(), 5_000);
        let detail = reg
            .detail("acme/kanban-team", "latest")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.version, "1.3.0");
        assert!(detail.author.verified);
        assert_eq!(detail.agents[0].model.as_deref(), Some("opus"));
        assert_eq!(detail.workers[0].resolved.as_deref(), Some("1.6.1"));
        assert!(reg
            .detail("acme/missing", "latest")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn http_registry_memoizes_blobs() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let sha = "a".repeat(64);
        Mock::given(method("GET"))
            .and(path(format!("/blobs/{sha}")))
            .respond_with(ResponseTemplate::new(200).set_body_string("# body\n"))
            .expect(1)
            .mount(&server)
            .await;
        let reg = HttpKitRegistry::new(&server.uri(), 5_000);
        assert_eq!(reg.blob(&sha).await.unwrap().as_deref(), Some("# body\n"));
        assert_eq!(reg.blob(&sha).await.unwrap().as_deref(), Some("# body\n"));
        assert_eq!(reg.blob("not-a-sha").await.unwrap(), None);
    }
}
