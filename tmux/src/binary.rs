//! Finding the real tmux.
//!
//! Release rules name this worker's binary `tmux`, the same name as the
//! program it drives. Wherever the engine puts the worker on `PATH`, a plain
//! `Command::new("tmux")` could run the worker itself, which would answer by
//! starting another worker. Resolution therefore walks `PATH` by hand, skips
//! this executable, and accepts a candidate only when `-V` answers like tmux.
//! The probe also sets [`PROBE_ENV`], which makes this binary exit at once
//! (see `main.rs`) if a copy of it is ever probed anyway. A bare name is looked
//! for in a few more folders after `PATH`, because a daemon's `PATH` is short
//! (simple-installs D5).

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
        return vec![expand_home(binary, std::env::var_os("HOME").as_deref())];
    }
    path_var
        .map(|value| {
            std::env::split_paths(value)
                .map(|dir| dir.join(binary))
                .collect()
        })
        .unwrap_or_default()
}

/// Where tools live that a short `PATH` (launchd, systemd, an SSH session)
/// leaves out: where an install puts tmux, Homebrew's two, Linuxbrew's, MacPorts'.
pub fn fallback_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    // An empty or relative `HOME` would mean a tmux in the current folder.
    home.filter(|home| home.is_absolute())
        .map(|home| home.join(".local/bin"))
        .into_iter()
        .chain(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/home/linuxbrew/.linuxbrew/bin",
                "/opt/local/bin",
            ]
            .map(PathBuf::from),
        )
        .collect()
}

/// [`candidates`], then the fallback folders for a bare name.
fn lookup(binary: &str, path_var: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut found = candidates(binary, path_var);
    if !binary.contains('/') {
        for candidate in fallback_dirs(home).iter().map(|dir| dir.join(binary)) {
            if !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

fn expand_home(raw: &str, home: Option<&std::ffi::OsStr>) -> PathBuf {
    let home = home.map(Path::new).filter(|home| home.is_absolute());
    match (raw.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(raw),
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

pub(crate) fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

async fn probe(path: &Path) -> Result<String, String> {
    probe_within(path, PROBE_TIMEOUT).await
}

/// `path -V` must answer like tmux within `limit`.
pub(crate) async fn probe_within(path: &Path, limit: Duration) -> Result<String, String> {
    let output = tokio::time::timeout(
        limit,
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
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut rejected = Vec::new();
    for candidate in lookup(binary, path_var.as_deref(), home.as_deref()) {
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

    #[cfg(test)]
    pub(crate) fn seed(&self, binary: &str, result: Result<Resolved, String>) {
        *self
            .entry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Entry {
            binary: binary.to_string(),
            result,
            at: Instant::now(),
        });
    }

    /// Drop what is known, so the next [`Self::get`] looks again.
    pub fn forget(&self) {
        *self
            .entry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
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

    #[test]
    fn a_bare_name_is_also_looked_for_in_the_fallback_folders() {
        let path = std::env::join_paths(["/a", "/usr/local/bin"]).unwrap();
        assert_eq!(
            lookup("tmux", Some(&path), Some(Path::new("/h"))),
            vec![
                PathBuf::from("/a/tmux"),
                PathBuf::from("/usr/local/bin/tmux"),
                PathBuf::from("/h/.local/bin/tmux"),
                PathBuf::from("/opt/homebrew/bin/tmux"),
                PathBuf::from("/home/linuxbrew/.linuxbrew/bin/tmux"),
                PathBuf::from("/opt/local/bin/tmux"),
            ]
        );
        assert_eq!(
            lookup("tmux", None, None).first(),
            Some(&PathBuf::from("/opt/homebrew/bin/tmux"))
        );
    }

    #[test]
    fn a_home_that_isnt_absolute_never_means_the_current_folder() {
        for home in ["", "relative", "./x"] {
            let found = lookup("tmux", None, Some(Path::new(home)));
            assert_eq!(
                found.first(),
                Some(&PathBuf::from("/opt/homebrew/bin/tmux"))
            );
            assert!(found.iter().all(|path| path.is_absolute()), "{found:?}");
            assert_eq!(
                expand_home("~/bin/tmux", Some(std::ffi::OsStr::new(home))),
                PathBuf::from("~/bin/tmux")
            );
        }
        assert_eq!(
            expand_home("~/bin/tmux", Some(std::ffi::OsStr::new("/h"))),
            PathBuf::from("/h/bin/tmux")
        );
    }

    #[test]
    fn path_keeps_its_own_meaning_for_finding_tmux() {
        // What a shell would run, and what this worker has always done.
        let path = std::env::join_paths(["", "rel", "/a"]).unwrap();
        assert_eq!(
            candidates("tmux", Some(&path)),
            vec![
                PathBuf::from("tmux"),
                PathBuf::from("rel/tmux"),
                PathBuf::from("/a/tmux")
            ]
        );
    }

    #[test]
    fn a_configured_path_is_never_looked_for_elsewhere() {
        assert_eq!(
            lookup("/opt/tmux", None, Some(Path::new("/h"))),
            vec![PathBuf::from("/opt/tmux")]
        );
    }

    #[tokio::test]
    async fn a_forgotten_entry_is_looked_up_again() {
        let cache = BinaryCache::default();
        let dir = std::env::temp_dir().join(format!("iii-tmux-binary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-tmux");
        let binary = script.to_str().unwrap();
        assert!(cache.get(binary).await.is_err());

        std::fs::write(&script, "#!/bin/sh\necho 'tmux 3.7c'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(cache.get(binary).await.is_err(), "the miss is still cached");

        cache.forget();
        let mut resolved = cache.get(binary).await;
        for _ in 0..20 {
            // A fork elsewhere in the test process may still hold the new file open.
            if !resolved
                .as_ref()
                .is_err_and(|error| error.contains("os error 26"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            cache.forget();
            resolved = cache.get(binary).await;
        }
        assert_eq!(resolved.unwrap().version, "3.7c");
        let _ = std::fs::remove_dir_all(dir);
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
