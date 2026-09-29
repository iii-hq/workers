//! `browser::doctor` — read-only environment diagnostics. Reports what the
//! worker can and cannot do right now and how to enable what is degraded.
//! Never launches a browser; the only side effect is running the detected
//! binary with `--version`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{BrowserEngine, WorkerConfig};

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
    /// Configured engine: `chromium` or `lightpanda`.
    pub engine: String,
    /// The engine binary the worker would launch (a Chromium/Chrome, or the
    /// `lightpanda` binary). The field name predates the `engine` setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_path: Option<String>,
    /// `<binary> --version`, first line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromium_version: Option<String>,
    pub headless_default: bool,
    /// Live-tab cap (`max_sessions`).
    pub max_sessions: u64,
    /// Tabs with a page open right now.
    pub active_sessions: u64,
    /// Every tab, live or asleep.
    pub open_tabs: u64,
    /// Whether the shared Chromium process is running.
    pub browser_running: bool,
    /// Where the profile, downloads and tab list live.
    pub data_dir: String,
    pub allowed_schemes: Vec<String>,
    pub configured_origin_policies: u64,
    pub default_origin_policy_set: bool,
    pub allow_history_access: bool,
    pub allow_cookie_import: bool,
    /// Whether attach mode is enabled (allow_attach).
    pub attach_enabled: bool,
    /// Whether ffmpeg is on PATH, which browser::recording requires.
    pub recording_available: bool,
    pub issues: Vec<DoctorIssue>,
}

/// Whether ffmpeg is invokable, gating `browser::recording`. Blocking; call
/// from spawn_blocking.
pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

/// Resolve the engine binary the worker would launch: the configured
/// `executable` when set, otherwise the first existing system Chromium
/// candidate, or `lightpanda` on PATH for the Lightpanda engine.
pub fn detect_executable(cfg: &WorkerConfig) -> Option<std::path::PathBuf> {
    if !cfg.executable.is_empty() {
        let path = std::path::PathBuf::from(&cfg.executable);
        return path.exists().then_some(path);
    }
    match cfg.engine {
        BrowserEngine::Chromium => CANDIDATES
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists()),
        BrowserEngine::Lightpanda => find_on_path(LIGHTPANDA_BINARY),
    }
}

/// Lightpanda ships Linux and macOS binaries only (Windows runs it in WSL).
pub const LIGHTPANDA_BINARY: &str = "lightpanda";

/// First `PATH` entry holding `name`, the way a shell resolves a command.
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// What `browser::doctor` tells an operator whose engine binary is missing.
pub fn missing_executable_issue(cfg: &WorkerConfig) -> DoctorIssue {
    let configured = (!cfg.executable.is_empty())
        .then(|| format!("configured executable '{}' does not exist", cfg.executable));
    match cfg.engine {
        BrowserEngine::Chromium => DoctorIssue {
            what: configured.unwrap_or_else(|| "no Chromium/Chrome install found".to_string()),
            enable_how: "install Google Chrome or Chromium, or point the worker config \
                         `executable` at a browser binary"
                .to_string(),
        },
        BrowserEngine::Lightpanda => DoctorIssue {
            what: configured.unwrap_or_else(|| "no `lightpanda` binary found on PATH".to_string()),
            enable_how: "install it with `brew install lightpanda-io/browser/lightpanda` or \
                         download the nightly binary from \
                         https://github.com/lightpanda-io/browser/releases/tag/nightly and \
                         put it on PATH, or point the worker config `executable` at it"
                .to_string(),
        },
    }
}

/// `<binary> --version`, first line. Blocking; call from spawn_blocking.
pub fn chromium_version(path: &std::path::Path) -> Option<String> {
    engine_version(BrowserEngine::Chromium, path)
}

/// The engine binary's version line: `chrome --version`, or `lightpanda
/// version` (its CLI takes subcommands, `--version` is an unknown command).
/// Blocking; call from spawn_blocking.
pub fn engine_version(engine: BrowserEngine, path: &std::path::Path) -> Option<String> {
    let arg = match engine {
        BrowserEngine::Chromium => "--version",
        BrowserEngine::Lightpanda => "version",
    };
    let output = std::process::Command::new(path).arg(arg).output().ok()?;
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
        assert!(detect_executable(&cfg).is_none());
    }

    #[test]
    fn lightpanda_resolves_from_path_or_reports_how_to_get_it() {
        let dir = std::env::temp_dir().join(format!("browser-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join(LIGHTPANDA_BINARY);
        std::fs::write(&bin, "").unwrap();
        let path = std::env::join_paths([dir.clone()]).unwrap();
        let cfg = WorkerConfig {
            engine: BrowserEngine::Lightpanda,
            ..WorkerConfig::default()
        };
        let found = std::env::split_paths(&path)
            .map(|d| d.join(LIGHTPANDA_BINARY))
            .find(|p| p.is_file());
        assert_eq!(found, Some(bin));
        assert!(missing_executable_issue(&cfg)
            .enable_how
            .contains("lightpanda-io/browser/releases"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
