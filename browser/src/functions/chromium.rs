//! `browser::chromium::status` and `browser::chromium::install`: whether this
//! machine has a Chromium the worker can launch, and getting one when it does
//! not. The install itself lives in [`crate::chromium::install`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chromium::install::InstallProgress;
use crate::chromium::{self, Source};
use crate::config::WorkerConfig;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ChromiumStatusInput {}

/// Whether `browser::chromium::install` can download a Chromium here.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstallSupport {
    /// A Chrome for Testing build exists for this platform.
    pub supported: bool,
    /// Chrome for Testing platform: `linux64`, `linux-arm64`, `mac-arm64`,
    /// `mac-x64`, or `win64`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Where the download is kept (`III_BROWSER_CACHE_DIR`, else
    /// `${XDG_CACHE_HOME:-~/.cache}/iii/browser`, plus `/chrome`).
    pub dir: String,
    /// Roughly how big the download is.
    pub approx_download_mb: u64,
    /// Why there is no download here (unsupported platforms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChromiumStatusOutput {
    /// A Chromium the worker can launch exists.
    pub found: bool,
    /// The binary sessions launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `<binary> --version`, first line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where it came from: `configured`, `env`, `system`, `managed` (the
    /// download), `playwright`, or `puppeteer`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// False when sessions launch this Chromium with `--no-sandbox` (a
    /// cache copy on a Linux that blocks the user-namespace sandbox).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandboxed: Option<bool>,
    /// The configured interactive engine (`chromium` or `lightpanda`).
    /// Scrapling's browser tiers use Chromium either way.
    pub engine: String,
    /// The places looked, in order.
    pub searched: Vec<String>,
    pub install: InstallSupport,
    /// This worker's latest install job, running or finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<InstallProgress>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ChromiumInstallInput {
    /// Download even when a Chromium is already found (a system one, or
    /// the same version already downloaded).
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum InstallStatus {
    /// A new background job started; follow `job_id`.
    Started,
    /// A job was already running; `job_id` is that job.
    Running,
    /// Nothing to do: a Chromium is already found (`path`, `version`).
    AlreadyInstalled,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChromiumInstallOutput {
    /// The install job; its progress arrives on the
    /// `browser::chromium-install-progress` trigger and in
    /// `browser::chromium::status` (`job`). Absent for `already-installed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub status: InstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// What `install.supported` says for this machine.
pub fn install_support() -> InstallSupport {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let platform = chromium::cft_platform(os, arch);
    InstallSupport {
        supported: platform.is_some(),
        platform: platform.map(str::to_string),
        dir: chromium::install_dir().display().to_string(),
        approx_download_mb: chromium::APPROX_DOWNLOAD_MB,
        reason: platform
            .is_none()
            .then(|| chromium::unsupported_reason(os, arch)),
    }
}

/// The status report. Blocking (runs the binary with `--version`); call
/// from spawn_blocking.
pub fn status(cfg: &WorkerConfig) -> ChromiumStatusOutput {
    let env = chromium::SearchEnv::current(cfg);
    let resolved = env.resolve();
    let version = resolved
        .as_ref()
        .and_then(|r| crate::functions::doctor::chromium_version(&r.path));
    ChromiumStatusOutput {
        found: resolved.is_some(),
        path: resolved.as_ref().map(|r| r.path.display().to_string()),
        version,
        source: resolved.as_ref().map(|r| r.source),
        sandboxed: resolved.as_ref().map(|r| !chromium::needs_no_sandbox(r)),
        engine: cfg.engine.as_str().to_string(),
        searched: env.searched(),
        install: install_support(),
        job: chromium::install::latest_job(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_status_is_kebab_case() {
        let out = ChromiumInstallOutput {
            job_id: None,
            status: InstallStatus::AlreadyInstalled,
            path: Some("/usr/bin/chromium".to_string()),
            version: None,
        };
        let v = serde_json::to_value(out).unwrap();
        assert_eq!(v["status"], "already-installed");
        assert!(v.get("job_id").is_none());
    }

    #[test]
    fn install_support_matches_the_platform_table() {
        let support = install_support();
        assert_eq!(
            support.supported,
            chromium::host_platform().is_some(),
            "{support:?}"
        );
        assert_eq!(support.supported, support.reason.is_none());
        assert!(support.dir.ends_with("chrome"), "{}", support.dir);
        assert_eq!(support.approx_download_mb, chromium::APPROX_DOWNLOAD_MB);
    }

    #[test]
    fn install_input_defaults_to_no_force() {
        let input: ChromiumInstallInput = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!input.force);
    }
}
