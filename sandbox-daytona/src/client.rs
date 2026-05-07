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

    /// Boot a new sandbox. TODO: verify path + body shape against Daytona's
    /// production REST surface — current path is a placeholder.
    pub async fn create(
        &self,
        image: &str,
        idle_timeout_secs: u64,
    ) -> Result<CreatedSandbox, WorkerError> {
        let _ = (image, idle_timeout_secs);
        let _builder = self.auth(self.http.post(self.url("/sandboxes")));
        // TODO: send + parse, then map status with crate::map_http_status.
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona POST /sandboxes".to_string(),
        ))
    }

    pub async fn exec(
        &self,
        sandbox_id: &str,
        cmd: &str,
        args: &[String],
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, WorkerError> {
        let _ = (sandbox_id, cmd, args, timeout_ms);
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona exec endpoint".to_string(),
        ))
    }

    pub async fn stop(&self, sandbox_id: &str) -> Result<(), WorkerError> {
        let _ = sandbox_id;
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona DELETE /sandboxes/{id}".to_string(),
        ))
    }

    pub async fn list(&self) -> Result<Vec<crate::SandboxRecord>, WorkerError> {
        Err(WorkerError::ProviderUnavailable(
            "TODO: wire Daytona GET /sandboxes".to_string(),
        ))
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
