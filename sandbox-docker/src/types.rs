use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct SandboxConfig {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub memory: Option<u64>,
    #[serde(default)]
    pub cpu: Option<f64>,
    #[serde(default)]
    pub network: Option<bool>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub workdir: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Sandbox {
    pub id: String,
    pub image: String,
    pub status: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub config: SandboxConfig,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_directory: bool,
}
