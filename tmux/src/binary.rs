//! Finding the real tmux.
//!
//! Release rules name this worker's binary `tmux`, the same name as the
//! program it drives. Wherever the engine puts the worker on `PATH`, a plain
//! `Command::new("tmux")` could run the worker itself, which would answer by
//! starting another worker. Resolution therefore walks `PATH` by hand, skips
//! this executable, and accepts a candidate only when `-V` answers like tmux.
//! The probe also sets [`PROBE_ENV`], which makes this binary exit at once
//! (see `main.rs`) if a copy of it is ever probed anyway.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Set on the `-V` probe; this worker's `main` exits immediately when it sees it.
pub const PROBE_ENV: &str = "III_TMUX_WORKER_PROBE";

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_FAILED_AFTER: Duration = Duration::from_secs(15);
const RECHECK_FOUND_AFTER: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub version: String,
}

/// Candidate paths for `binary`, in lookup order: the path itself when it
/// names one, otherwise every `PATH` entry joined with it.
pub fn candidates(binary: &str, path_var: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    if binary.contains('/') {
        return vec![expand_home(binary)];
    }
    path_var
        .map(|value| {
            std::env::split_paths(value)
                .map(|dir| dir.join(binary))
                .collect()
        })
        .unwrap_or_default()
}

fn expand_home(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(raw)),
        None => PathBuf::from(raw),
    }
}

/// `tmux 3.6` → `3.6`; `tmux next-3.7` → `next-3.7`. Anything else is not tmux.
pub fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    let version = line.strip_prefix("tmux ")?.trim();
    (!version.is_empty() && !version.contains(char::is_whitespace)).then(|| version.to_string())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

async fn probe(path: &Path) -> Result<String, String> {
    let output = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::process::Command::new(path)
            .arg("-V")
            .env(PROBE_ENV, "1")
            .env_remove("TMUX")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{} -V timed out", path.display()))?
    .map_err(|error| format!("{} -V failed: {error}", path.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match (output.status.success(), parse_version(&stdout)) {
        (true, Some(version)) => Ok(version),
        _ => Err(format!("{} does not answer -V like tmux", path.display())),
    }
}

/// Resolve `binary` to a real tmux, never this worker's own executable.
pub async fn resolve(binary: &str) -> Result<Resolved, String> {
    let own = std::env::current_exe().ok();
    let path_var = std::env::var_os("PATH");
    let mut rejected = Vec::new();
    for candidate in candidates(binary, path_var.as_deref()) {
        if !is_executable_file(&candidate) {
            continue;
        }
        if own.as_deref().is_some_and(|own| same_file(own, &candidate)) {
            rejected.push(format!("{} is this worker", candidate.display()));
            continue;
        }
        match probe(&candidate).await {
            Ok(version) => {
                return Ok(Resolved {
                    path: candidate,
                    version,
                })
            }
            Err(error) => rejected.push(error),
        }
    }
    if rejected.is_empty() {
        Err(format!(
            "tmux is not installed ('{binary}' not found on PATH)"
        ))
    } else {
        Err(format!(
            "no usable tmux for '{binary}': {}",
            rejected.join("; ")
        ))
    }
}

struct Entry {
    binary: String,
    result: Result<Resolved, String>,
    at: Instant,
}

/// Resolution result per configured binary: a miss is retried after a few
/// seconds (tmux may be installed while the worker runs), a hit after ten
/// minutes (an upgrade may move it).
#[derive(Default)]
pub struct BinaryCache {
    entry: Mutex<Option<Entry>>,
}

impl BinaryCache {
    pub async fn get(&self, binary: &str) -> Result<Resolved, String> {
        if let Some(result) = self.cached(binary) {
            return result;
        }
        let result = resolve(binary).await;
        if let Err(error) = &result {
            tracing::warn!(%error, "tmux binary unavailable");
        }
        *self
            .entry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Entry {
            binary: binary.to_string(),
            result: result.clone(),
            at: Instant::now(),
        });
        result
    }

    fn cached(&self, binary: &str) -> Option<Result<Resolved, String>> {
        let guard = self
            .entry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.as_ref().filter(|entry| entry.binary == binary)?;
        let ttl = if entry.result.is_ok() {
            RECHECK_FOUND_AFTER
        } else {
            RETRY_FAILED_AFTER
        };
        (entry.at.elapsed() < ttl).then(|| entry.result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_like_tmux_prints_them() {
        assert_eq!(parse_version("tmux 3.6\n").as_deref(), Some("3.6"));
        assert_eq!(parse_version("tmux next-3.7").as_deref(), Some("next-3.7"));
        assert_eq!(parse_version("tmux 3.3a").as_deref(), Some("3.3a"));
        assert_eq!(parse_version("tmux-worker 0.1.0"), None);
        assert_eq!(parse_version("error: unexpected argument '-V'"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn candidates_walk_path_unless_a_path_is_given() {
        let path = std::env::join_paths(["/a", "/b"]).unwrap();
        assert_eq!(
            candidates("tmux", Some(&path)),
            vec![PathBuf::from("/a/tmux"), PathBuf::from("/b/tmux")]
        );
        assert_eq!(
            candidates("/opt/tmux/bin/tmux", Some(&path)),
            vec![PathBuf::from("/opt/tmux/bin/tmux")]
        );
    }

    #[tokio::test]
    async fn never_resolves_to_this_executable() {
        let own = std::env::current_exe().unwrap();
        let error = resolve(own.to_str().unwrap()).await.unwrap_err();
        assert!(error.contains("is this worker"), "{error}");
    }

    #[tokio::test]
    async fn a_program_that_is_not_tmux_is_rejected() {
        let error = resolve("/bin/echo").await.unwrap_err();
        assert!(error.contains("does not answer -V like tmux"), "{error}");
    }
}
