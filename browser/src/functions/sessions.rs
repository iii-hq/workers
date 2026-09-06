//! `browser::sessions::start` / `list` / `stop` — tab lifecycle. A session is
//! a tab in the worker's browser: it stays open until stopped (or until its
//! optional `ttl_ms` elapses), sleeps after `inactive_after_ms` unused and
//! unwatched, and wakes on the next call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInput {
    /// URL to open immediately. Omit to start on about:blank.
    #[serde(default)]
    pub url: Option<String>,
    /// Force a visible window, overriding the configured `headless` default.
    /// Applies when this call launches the browser process; a browser that
    /// is already running keeps its mode.
    #[serde(default)]
    pub headful: Option<bool>,
    /// Inspection-only session for its whole lifetime: act, evaluate, execute
    /// and styles::write are rejected; navigation and reads work.
    #[serde(default)]
    pub read_only: Option<bool>,
    /// INCOGNITO TAB. Opens the tab in a private browser context: it shares
    /// no cookies, logins, or storage with the regular tabs, nothing it does
    /// is saved to disk (no cookies, no history, no tab record), it does not
    /// come back after a restart, and inactivity closes it for good instead
    /// of putting it to sleep. Everything lives in memory for as long as the
    /// tab does. Use it for logins you do not want kept, or to see a site
    /// signed out.
    #[serde(default)]
    pub incognito: Option<bool>,
    /// Optional lifetime in milliseconds: the tab closes on its own this
    /// long after it opened, even while in use. Omit for a tab that stays
    /// until stopped (what the console does).
    #[serde(default)]
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StartOutput {
    /// Pass this to every other browser function.
    pub session_id: String,
    pub url: String,
    pub headless: bool,
    pub read_only: bool,
    /// True for a private tab; see `incognito` on the request.
    pub incognito: bool,
    /// Chromium's error text when the requested url did not load and the tab
    /// shows the browser's error page instead (a network failure, or an
    /// empty HTTP error response such as a 400). Absent when the page came
    /// up. The tab is open either way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    pub read_only: bool,
    /// Private tab: nothing persisted, closes instead of sleeping.
    pub incognito: bool,
    /// True while the tab has its page open. A sleeping tab (false) is
    /// listed and usable; the next call on it reopens the page at `url`.
    pub active: bool,
    /// Lifetime the tab was opened with, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    pub created_ms: i64,
    pub last_used_ms: i64,
    /// Console entries captured since the page opened; 0 while asleep.
    pub console_entries: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopInput {
    /// Tab to close. Closing an unknown or already-closed id succeeds.
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
        // `file` ships enabled so a local document can be rendered.
        assert!(check_scheme(&cfg, "file:///tmp/report.pdf").is_ok());
        assert!(check_scheme(&cfg, "chrome://settings").is_err());
        assert!(check_scheme(&cfg, "not a url").is_err());
    }

    /// The list is what gates navigation, so an operator narrowing it has to
    /// actually close the door — including on the scheme that now ships open.
    #[test]
    fn a_narrowed_list_still_refuses_what_it_drops() {
        let cfg = WorkerConfig {
            allowed_schemes: vec!["https".to_string()],
            ..WorkerConfig::default()
        };
        assert!(check_scheme(&cfg, "https://example.com").is_ok());
        assert!(check_scheme(&cfg, "file:///etc/passwd").is_err());
        assert!(check_scheme(&cfg, "http://example.com").is_err());
    }
}
