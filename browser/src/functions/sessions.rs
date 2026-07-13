//! `browser::sessions::start` / `list` / `stop` — session lifecycle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInput {
    /// URL to open immediately. Omit to start on about:blank.
    #[serde(default)]
    pub url: Option<String>,
    /// Force a visible window for this session, overriding the configured
    /// `headless` default.
    #[serde(default)]
    pub headful: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StartOutput {
    /// Pass this to every other browser function.
    pub session_id: String,
    pub url: String,
    pub headless: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionInfo {
    pub session_id: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub headless: bool,
    pub created_ms: i64,
    pub last_used_ms: i64,
    pub console_entries: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopInput {
    /// Session to stop. Stopping an unknown or already-stopped id succeeds.
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopOutput {
    pub ok: bool,
    /// False when the session was already gone.
    pub was_running: bool,
}

/// Scheme allowlist check shared by `sessions::start` and `navigate`.
pub fn check_scheme(cfg: &WorkerConfig, raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("invalid url: {e}"))?;
    let scheme = parsed.scheme();
    if cfg.allowed_schemes.iter().any(|s| s == scheme) {
        Ok(())
    } else {
        Err(format!(
            "scheme '{scheme}' is not allowed (allowed: {})",
            cfg.allowed_schemes.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_allowlist() {
        let cfg = WorkerConfig::default();
        assert!(check_scheme(&cfg, "http://localhost:3000").is_ok());
        assert!(check_scheme(&cfg, "https://example.com/a?b=c").is_ok());
        assert!(check_scheme(&cfg, "file:///etc/passwd").is_err());
        assert!(check_scheme(&cfg, "chrome://settings").is_err());
        assert!(check_scheme(&cfg, "not a url").is_err());
    }
}
