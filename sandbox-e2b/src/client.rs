//! Narrow reqwest wrapper for the E2B REST API. Holds the base URL, api key,
//! and a small helper for building requests. Endpoint paths and bodies are
//! still stubbed pending a verified pass against the live E2B API; every
//! call below returns `WorkerError::ProviderUnavailable` with a TODO marker.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::WorkerError;

#[derive(Debug, Clone)]
pub struct E2bClient {
    pub api_base: String,
    pub api_key: String,
    pub http: Client,
}

impl E2bClient {
    pub fn new(api_base: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            api_key: api_key.into(),
            http: Client::builder()
                .user_agent("iii-sandbox-e2b/0.1")
                .build()
                .expect("reqwest client"),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base.trim_end_matches('/'), path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("X-API-KEY", &self.api_key)
    }

    /// Boot a new E2B sandbox. POST /sandboxes
    /// Body: {templateID, autoPause, timeout}. `timeout` is the lifetime
    /// after which E2B auto-kills the sandbox unless extended; without it
    /// the platform applies a short default (~15 s) that's unusable for
    /// real workloads. We forward `idle_timeout_secs` from the caller.
    /// Response: {sandboxID, templateID, alias, clientID, envdVersion}.
    pub async fn create(
        &self,
        image: &str,
        idle_timeout_secs: u64,
    ) -> Result<CreatedSandbox, WorkerError> {
        let body = serde_json::json!({
            "templateID": image,
            "autoPause": false,
            "timeout": idle_timeout_secs,
        });
        let resp = self
            .auth(self.http.post(self.url("/sandboxes")))
            .json(&body)
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: E2bCreateResponse = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(CreatedSandbox {
            sandbox_id: parsed.sandbox_id,
            image: parsed.alias.unwrap_or(parsed.template_id),
            started_at: unix_now_secs(),
        })
    }

    pub async fn exec(
        &self,
        sandbox_id: &str,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, WorkerError> {
        let _ = (sandbox_id, cmd, args, timeout_ms);
        // Exec on E2B runs through /sandboxes/{id}/processes which streams
        // results back as SSE events. Wiring it cleanly needs a streaming
        // parser; left for the next iteration.
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire E2B /sandboxes/{id}/processes streaming exec".to_string(),
        ))
    }

    /// Tear down a sandbox. DELETE /sandboxes/{id}.
    ///
    /// 404 is treated as success: `stop` is idempotent — the post-state
    /// (sandbox is not running) is already achieved when the upstream
    /// reports it doesn't exist. E2B auto-kills sandboxes once their
    /// `endAt` passes, so a stop racing the reaper would otherwise
    /// surface as a spurious S502 to the caller and leak the worker's
    /// in-flight counter.
    pub async fn stop(&self, sandbox_id: &str) -> Result<(), WorkerError> {
        let resp = self
            .auth(
                self.http
                    .delete(self.url(&format!("/sandboxes/{sandbox_id}"))),
            )
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) || status == 404 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(crate::map_http_status(status, &text))
    }

    pub async fn list(&self) -> Result<Vec<crate::SandboxRecord>, WorkerError> {
        let resp = self
            .auth(self.http.get(self.url("/sandboxes")))
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: Vec<E2bListItem> = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(parsed
            .into_iter()
            .map(|it| crate::SandboxRecord {
                sandbox_id: it.sandbox_id,
                image: it.alias.unwrap_or(it.template_id),
                started_at: it.started_at,
            })
            .collect())
    }

    pub async fn snapshot(&self, sandbox_id: &str) -> Result<String, WorkerError> {
        let _ = sandbox_id;
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire E2B pause/snapshot endpoint".to_string(),
        ))
    }

    pub async fn expose_port(&self, sandbox_id: &str, port: u16) -> Result<String, WorkerError> {
        let _ = (sandbox_id, port);
        Err(WorkerError::ProviderUnavailable(
            "TODO: derive E2B port URL".to_string(),
        ))
    }

    pub async fn fs_read(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, WorkerError> {
        let _ = (sandbox_id, path);
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire E2B fs read".to_string(),
        ))
    }

    pub async fn fs_write(
        &self,
        sandbox_id: &str,
        path: &str,
        bytes: &[u8],
        mode: Option<u32>,
    ) -> Result<(), WorkerError> {
        let _ = (sandbox_id, path, bytes, mode);
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire E2B fs write".to_string(),
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct E2bCreateResponse {
    #[serde(rename = "sandboxID")]
    sandbox_id: String,
    #[serde(rename = "templateID")]
    template_id: String,
    alias: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct E2bListItem {
    #[serde(rename = "sandboxID")]
    sandbox_id: String,
    #[serde(rename = "templateID")]
    template_id: String,
    alias: Option<String>,
    #[serde(rename = "startedAt", default)]
    started_at: String,
}

fn unix_now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedSandbox {
    pub sandbox_id: String,
    pub image: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}
