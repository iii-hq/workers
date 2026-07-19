//! `browser::doctor` — read-only environment diagnostics. Reports what the
//! worker can and cannot do right now and how to enable what is degraded.
//! Never launches a browser; the only side effect is running the detected
//! binary with `--version`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DoctorInput {}

/// One degraded capability plus the way to enable it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DoctorIssue {
    pub what: String,
    pub enable_how: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DoctorOutput {
    /// True when sessions can start right now.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_version: Option<String>,
    pub headless_default: bool,
    pub max_sessions: u64,
    pub active_sessions: u64,
    pub allowed_schemes: Vec<String>,
    pub issues: Vec<DoctorIssue>,
}

/// Candidate system installs checked when config `executable` is empty, in
/// order. Mirrors the auto-detection the launcher performs.
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/microsoft-edge",
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
];

/// Resolve the Chromium binary the worker would launch: the configured
/// `executable` when set, otherwise the first existing system candidate.
pub fn detect_chromium(cfg: &WorkerConfig) -> Option<std::path::PathBuf> {
    if !cfg.executable.is_empty() {
        let path = std::path::PathBuf::from(&cfg.executable);
        return path.exists().then_some(path);
    }
    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

/// `<binary> --version`, first line. Blocking; call from spawn_blocking.
pub fn chromium_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_missing_executable_is_none() {
        let cfg = WorkerConfig {
            executable: "/definitely/not/a/real/chromium".to_string(),
            ..WorkerConfig::default()
        };
        assert!(detect_chromium(&cfg).is_none());
    }
}
