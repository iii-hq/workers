//! Locate the operator-provided `cloudflared` and report it without starting a
//! tunnel. quick-tunnel never downloads or installs it: the report points the
//! operator at Cloudflare's install page instead.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cloudflare's official cloudflared downloads page.
pub const INSTALL_URL: &str =
    "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/";

/// Searched after the worker's own PATH. The child PATH is deliberately narrow,
/// and Homebrew on Apple silicon installs outside it (`/opt/homebrew/bin`).
const EXTRA_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Prerequisites {
    pub cloudflared: CloudflaredCheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct CloudflaredCheck {
    /// True when an executable was found and answered `--version`.
    pub found: bool,
    /// Absolute path quick-tunnel runs.
    pub path: Option<String>,
    /// First line of `cloudflared --version`.
    pub version: Option<String>,
    /// Why it is not usable, when `found` is false.
    pub error: Option<String>,
    /// Where to install it from; quick-tunnel never installs it.
    pub install_url: String,
}

/// Resolve the configured name (or absolute path) to an executable.
pub fn resolve(configured: &str) -> Option<PathBuf> {
    if configured.contains('/') {
        let path = PathBuf::from(configured);
        return is_executable(&path).then_some(path);
    }
    let own_path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&own_path)
        .chain(EXTRA_DIRS.iter().map(PathBuf::from))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(configured))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Lifecycle error for a missing executable, with the install pointer.
pub fn not_found_error(configured: &str) -> String {
    format!("cloudflared not found ({configured}); install it from {INSTALL_URL}")
}

/// Probe the executable with `--version` (bounded to five seconds).
pub async fn check(configured: &str) -> CloudflaredCheck {
    let missing = |error: String| CloudflaredCheck {
        found: false,
        path: None,
        version: None,
        error: Some(error),
        install_url: INSTALL_URL.into(),
    };
    let Some(path) = resolve(configured) else {
        return missing(not_found_error(configured));
    };
    let run = tokio::process::Command::new(&path)
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let shown = path.display().to_string();
    match tokio::time::timeout(Duration::from_secs(5), run).await {
        Ok(Ok(out)) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let version = text
                .lines()
                .next()
                .map(|line| line.trim().chars().take(200).collect::<String>())
                .filter(|line| !line.is_empty());
            CloudflaredCheck {
                found: true,
                path: Some(shown),
                version,
                error: None,
                install_url: INSTALL_URL.into(),
            }
        }
        Ok(Ok(out)) => CloudflaredCheck {
            path: Some(shown),
            ..missing(format!("cloudflared --version exited ({})", out.status))
        },
        Ok(Err(e)) => CloudflaredCheck {
            path: Some(shown),
            ..missing(format!("cloudflared --version failed ({:?})", e.kind()))
        },
        Err(_) => CloudflaredCheck {
            path: Some(shown),
            ..missing("cloudflared --version timed out".into())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_executable_points_at_the_install_page() {
        let check = check("/nonexistent/cloudflared").await;
        assert!(!check.found);
        assert!(check.error.unwrap().contains(INSTALL_URL));
        assert_eq!(check.install_url, INSTALL_URL);
        assert!(resolve("definitely-not-a-cloudflared-binary").is_none());
    }

    #[tokio::test]
    async fn found_executable_reports_path_and_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("cloudflared");
        std::fs::write(
            &exe,
            "#!/bin/sh\necho 'cloudflared version 2026.9.1 (built x)'\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let check = check(exe.to_str().unwrap()).await;
        assert!(check.found, "{check:?}");
        assert_eq!(
            check.version.as_deref(),
            Some("cloudflared version 2026.9.1 (built x)")
        );
        assert_eq!(check.path.as_deref(), exe.to_str());
    }
}
