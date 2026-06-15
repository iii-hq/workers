//! Operator config loaded from `config.yaml` (the seed default; same keys the
//! TS worker used). Missing file falls back to defaults; a malformed file is a
//! hard error so a typo fails the worker fast rather than silently.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub model: String,
    pub sandbox_mode: String,
    pub approval_policy: String,
    pub reasoning_effort: String,
    pub cwd: String,
    pub skip_git_repo_check: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            model: String::new(),
            sandbox_mode: "workspace-write".to_string(),
            approval_policy: "never".to_string(),
            reasoning_effort: String::new(),
            cwd: String::new(),
            skip_git_repo_check: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub engine_url: String,
    pub defaults: Defaults,
    pub events_stream: String,
    pub raw_events_stream: String,
    pub codex_executable: String,
    pub base_url: String,
    pub iii_context: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine_url: "ws://127.0.0.1:49134".to_string(),
            defaults: Defaults::default(),
            events_stream: "agent::events".to_string(),
            raw_events_stream: "codex::events".to_string(),
            codex_executable: String::new(),
            base_url: String::new(),
            iii_context: true,
        }
    }
}

impl Config {
    /// Load from a YAML file. A missing file yields defaults; any other error
    /// (parse, permissions) propagates so the worker fails fast.
    pub fn load(path: &str) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_yaml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }
}
