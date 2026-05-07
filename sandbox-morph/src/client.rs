//! Narrow reqwest wrapper for the Morph REST API. Holds the base URL, api key,
//! and a small helper for building requests. Endpoint paths and bodies are
//! still stubbed pending a verified pass against the live Morph API; every
//! call below returns `WorkerError::ProviderUnavailable` with a TODO marker.

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::WorkerError;

#[derive(Debug, Clone)]
pub struct MorphClient {
    pub api_base: String,
    pub api_key: String,
    pub http: Client,
}

impl MorphClient {
    pub fn new(api_base: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into(),
            api_key: api_key.into(),
            http: Client::builder()
                .user_agent("iii-sandbox-morph/0.1")
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

    /// Start a new Morph instance. POST /api/instance?snapshot_id=...
    /// Body: `{metadata, ttl_seconds, ttl_action}`. Morph requires a
    /// `snapshot_id` query parameter (not body) — we map the caller's
    /// `image` field into it. Without a registered snapshot the call
    /// fails upstream with a 4xx; this is the cost of Morph's
    /// snapshot-first model and is documented in the worker README.
    /// Response: `{id, status, refs:{snapshot_id}, ...}`.
    pub async fn create(
        &self,
        image: &str,
        idle_timeout_secs: u64,
    ) -> Result<CreatedSandbox, WorkerError> {
        if image.is_empty() {
            return Err(WorkerError::BadInput(
                "morph requires a non-empty `image` (snapshot id)".into(),
            ));
        }
        let body = serde_json::json!({
            "metadata": {},
            "ttl_seconds": idle_timeout_secs,
            "ttl_action": "stop",
        });
        let resp = self
            .auth(
                self.http
                    .post(self.url("/instance"))
                    .query(&[("snapshot_id", image)]),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: MorphInstance = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(CreatedSandbox {
            sandbox_id: parsed.id,
            image: parsed
                .refs
                .and_then(|r| r.snapshot_id)
                .unwrap_or_else(|| image.to_string()),
            started_at: unix_now_secs(),
        })
    }

    /// Run a command. POST /api/instance/{id}/exec body `{command: [...]}`.
    /// Morph wraps argv as a single array; we prepend the program name.
    pub async fn exec(
        &self,
        sandbox_id: &str,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, WorkerError> {
        let _ = timeout_ms; // Morph exec is synchronous; no per-call timeout knob today.
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push(cmd.to_string());
        command.extend(args.iter().cloned());
        let body = serde_json::json!({ "command": command });
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/instance/{sandbox_id}/exec"))),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: MorphExecResponse = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(ExecResult {
            stdout: parsed.stdout.unwrap_or_default(),
            stderr: parsed.stderr.unwrap_or_default(),
            exit_code: parsed.exit_code.unwrap_or(0),
            timed_out: false,
        })
    }

    /// Stop an instance. POST /api/instance/{id}/stop. 404/409 are
    /// treated as success — the post-state is what the caller wanted.
    pub async fn stop(&self, sandbox_id: &str) -> Result<(), WorkerError> {
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/instance/{sandbox_id}/stop"))),
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
            .auth(self.http.get(self.url("/instance")))
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: Vec<MorphInstance> = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(parsed
            .into_iter()
            .map(|it| crate::SandboxRecord {
                sandbox_id: it.id,
                image: it.refs.and_then(|r| r.snapshot_id).unwrap_or_default(),
                started_at: it.created_at.unwrap_or_default(),
            })
            .collect())
    }

    /// POST /api/instance/{id}/snapshot. Returns a snapshot id from
    /// the response body's `id` field.
    pub async fn snapshot(&self, sandbox_id: &str) -> Result<String, WorkerError> {
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/instance/{sandbox_id}/snapshot"))),
            )
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: MorphSnapshot = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(parsed.id)
    }

    pub async fn expose_port(&self, sandbox_id: &str, port: u16) -> Result<String, WorkerError> {
        let _ = (sandbox_id, port);
        // Morph exposes ports via `instance.expose_http_service(name, port)`
        // which doesn't have a single REST equivalent in the public docs;
        // wiring it cleanly needs more API exploration. Leaving stubbed.
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Morph expose_http_service".to_string(),
        ))
    }

    pub async fn fs_read(&self, sandbox_id: &str, path: &str) -> Result<Vec<u8>, WorkerError> {
        let _ = (sandbox_id, path);
        // Morph filesystem ops go through SSH, not REST; the worker
        // does not advertise `fs` capability so callers shouldn't hit
        // this path. Returning S502 as a defensive default.
        Err(WorkerError::ProviderUnavailable(
            "morph fs ops via SSH, not registered as iii capability".to_string(),
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
            "morph fs ops via SSH, not registered as iii capability".to_string(),
        ))
    }

    /// Branch a running instance into N siblings.
    /// POST /api/instance/{id}/branch?count=N — Morph's Infinibranch.
    /// Response shape: `{snapshot, instances: [...]}` per the SDK; we
    /// surface only the new instance ids.
    pub async fn branch(&self, sandbox_id: &str, count: u32) -> Result<Vec<String>, WorkerError> {
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/instance/{sandbox_id}/branch")))
                    .query(&[("count", count.to_string())]),
            )
            .json(&serde_json::json!({
                "snapshot_metadata": {},
                "instance_metadata": {},
            }))
            .send()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("send failed: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::map_http_status(status, &text));
        }
        let parsed: MorphBranchResponse = resp
            .json()
            .await
            .map_err(|e| WorkerError::ProviderUnavailable(format!("parse failed: {e}")))?;
        Ok(parsed
            .instances
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.id)
            .collect())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MorphInstance {
    id: String,
    refs: Option<MorphRefs>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MorphRefs {
    snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MorphExecResponse {
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
struct MorphSnapshot {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MorphBranchResponse {
    instances: Option<Vec<MorphInstance>>,
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
