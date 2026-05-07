//! Narrow reqwest wrapper for the Daytona REST API. Holds the base URL, api key,
//! and a small helper for building requests. Endpoint paths and bodies are
//! still stubbed pending a verified pass against the live Daytona API; every
//! call below returns `WorkerError::ProviderUnavailable` with a TODO marker.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::WorkerError;

#[derive(Debug, Clone)]
pub struct DaytonaClient {
    pub api_base: String,
    pub api_key: String,
    pub http: Client,
}

impl DaytonaClient {
    pub fn new(api_base: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            api_key: api_key.into(),
            http: Client::builder()
                .user_agent("iii-sandbox-daytona/0.1")
                .build()
                .expect("reqwest client"),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base.trim_end_matches('/'), path)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.api_key)
    }

    /// Boot a new Daytona sandbox. POST /sandbox.
    /// Body: `{snapshot?, autoStopInterval}`. `autoStopInterval` is in
    /// **minutes** (we round up from seconds). When `image` is empty or
    /// not a registered Daytona snapshot name we omit the field and let
    /// Daytona pick its default (`daytonaio/sandbox:0.6.0`).
    /// Response: `{id, snapshot, state, createdAt, ...}`.
    pub async fn create(
        &self,
        image: &str,
        idle_timeout_secs: u64,
    ) -> Result<CreatedSandbox, WorkerError> {
        // Daytona expects whole minutes. Round up so callers always get
        // at least the lifetime they asked for.
        let auto_stop_minutes = idle_timeout_secs.div_ceil(60).max(1);
        let mut body = serde_json::json!({
            "autoStopInterval": auto_stop_minutes,
        });
        if !image.is_empty() && image != "default" {
            body["snapshot"] = serde_json::Value::String(image.to_string());
        }
        let resp = self
            .auth(self.http.post(self.url("/sandbox")))
            .json(&body)
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: DaytonaSandbox = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(CreatedSandbox {
            sandbox_id: parsed.id,
            image: parsed.snapshot.unwrap_or_default(),
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
        // Daytona's process API runs through /sandbox/{id}/process which
        // streams stdout/stderr; wiring it cleanly needs a streaming
        // parser. Left for the next iteration — the worker still
        // registers the function so callers see a consistent ABI.
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona /sandbox/{id}/process streaming exec".to_string(),
        ))
    }

    /// Tear down a sandbox. DELETE /sandbox/{id}.
    ///
    /// 404 and 409 are both treated as success — `stop` is idempotent
    /// from the caller's view, and Daytona uses 409 when a previous
    /// delete is still mid-flight (the sandbox is on its way out, just
    /// not done yet). Surfacing either as S502 was the bug a live test
    /// caught: a second `stop` racing the platform's async deletion
    /// would falsely look like a provider failure and leak the in-flight
    /// counter.
    pub async fn stop(&self, sandbox_id: &str) -> Result<(), WorkerError> {
        let resp = self
            .auth(
                self.http
                    .delete(self.url(&format!("/sandbox/{sandbox_id}"))),
            )
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) || status == 404 || status == 409 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(crate::map_http_status(status, &text))
    }

    pub async fn list(&self) -> Result<Vec<crate::SandboxRecord>, WorkerError> {
        let resp = self
            .auth(self.http.get(self.url("/sandbox")))
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: Vec<DaytonaSandbox> = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(parsed
            .into_iter()
            .map(|it| crate::SandboxRecord {
                sandbox_id: it.id,
                image: it.snapshot.unwrap_or_default(),
                started_at: it.created_at_iso.unwrap_or_default(),
            })
            .collect())
    }

    pub async fn snapshot(&self, sandbox_id: &str) -> Result<String, WorkerError> {
        let _ = sandbox_id;
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona pause/snapshot endpoint".to_string(),
        ))
    }

    pub async fn expose_port(&self, sandbox_id: &str, port: u16) -> Result<String, WorkerError> {
        let _ = (sandbox_id, port);
        Err(WorkerError::ProviderUnavailable(
            "TODO: derive Daytona port URL".to_string(),
        ))
    }

    pub async fn fs_read(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, WorkerError> {
        let _ = (sandbox_id, path);
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona fs read".to_string(),
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
            "TODO: wire Daytona fs write".to_string(),
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DaytonaSandbox {
    id: String,
    snapshot: Option<String>,
    /// Daytona returns ISO8601 in `createdAt`. We pass it through as a
    /// string in `SandboxRecord`. `CreatedSandbox.started_at` uses the
    /// worker's local clock at response time; close enough.
    #[serde(rename = "createdAt", default)]
    created_at_iso: Option<String>,
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
