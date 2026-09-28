//! Locate the operator-provided `cloudflared` and report it without starting a
//! tunnel. quick-tunnel never downloads or installs it: the report points the
//! operator at Cloudflare's install page instead.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Cloudflare's official cloudflared downloads page.
pub const INSTALL_URL: &str =
    "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/";

/// The only directories a bare name is resolved in. The worker's inherited PATH
/// is deliberately NOT searched: a user-writable entry (~/.local/bin, shims)
/// could shadow the binary that publishes a local service to the Internet.
/// Anything else must be configured as an absolute path.
pub const TRUSTED_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

/// How long a successful `--version` probe of an unchanged binary is reused.
const PROBE_TTL: Duration = Duration::from_secs(60);

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
    TRUSTED_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(configured))
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

/// Reuses a successful probe while the resolved binary is unchanged, so status
/// callers do not spawn `cloudflared --version` on every read. A missing binary
/// is re-resolved every time (cheap, no process), so installing it shows up at
/// once.
#[derive(Default)]
pub struct ProbeCache {
    slot: tokio::sync::Mutex<Option<CachedProbe>>,
}

struct CachedProbe {
    path: PathBuf,
    modified: Option<SystemTime>,
    at: Instant,
    result: CloudflaredCheck,
}

impl ProbeCache {
    pub async fn check(&self, configured: &str) -> CloudflaredCheck {
        let Some(path) = resolve(configured) else {
            return check(configured).await;
        };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut slot = self.slot.lock().await;
        if let Some(cached) = slot.as_ref() {
            if cached.path == path
                && cached.modified == modified
                && cached.at.elapsed() < PROBE_TTL
                && cached.result.found
            {
                return cached.result.clone();
            }
        }
        let fresh = check(configured).await;
        *slot = Some(CachedProbe {
            path,
            modified,
            at: Instant::now(),
            result: fresh.clone(),
        });
        fresh
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

    #[test]
    fn bare_names_resolve_only_in_trusted_directories() {
        use std::os::unix::fs::PermissionsExt;
        // An executable outside the trusted list is never picked up by name.
        let dir = tempfile::tempdir().unwrap();
        let name = "quick-tunnel-shadow-cloudflared-test";
        let exe = dir.path().join(name);
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(resolve(name).is_none());
        // The same file is used when configured by absolute path.
        assert_eq!(resolve(exe.to_str().unwrap()), Some(exe));
    }

    #[tokio::test]
    async fn probe_cache_reuses_an_unchanged_binary_and_reprobes_a_changed_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let exe = dir.path().join("cloudflared");
        let script = |version: &str| {
            format!(
                "#!/bin/sh\necho run >> {}\necho 'cloudflared version {version}'\n",
                runs.display()
            )
        };
        std::fs::write(&exe, script("1")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = ProbeCache::default();
        let path = exe.to_str().unwrap();
        assert_eq!(
            cache.check(path).await.version.as_deref(),
            Some("cloudflared version 1")
        );
        assert_eq!(
            cache.check(path).await.version.as_deref(),
            Some("cloudflared version 1")
        );
        let count = || std::fs::read_to_string(&runs).unwrap().lines().count();
        assert_eq!(count(), 1, "second read served from cache");
        // A changed binary (new mtime) is probed again.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&exe, script("2")).unwrap();
        let changed = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&exe)
            .unwrap()
            .set_modified(changed)
            .unwrap();
        assert_eq!(
            cache.check(path).await.version.as_deref(),
            Some("cloudflared version 2")
        );
        assert_eq!(count(), 2);
    }
}
