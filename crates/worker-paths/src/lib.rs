//! Resolves worker-owned paths against the Compose project directory.
//!
//! Compose-managed workers receive `III_COMPOSE_DIR`. Standalone workers use
//! their process working directory. Configuration defaults stay relative so
//! serialized worker configuration remains portable. Absolute paths and
//! explicit home-relative paths keep their usual meaning.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Environment variable that contains the canonical Compose project directory.
pub const COMPOSE_DIR_ENV: &str = "III_COMPOSE_DIR";

/// Builds a portable default for serialized worker configuration.
///
/// Resolve the returned value with [`resolve_path`] before filesystem access.
pub fn default_path(relative: impl AsRef<Path>) -> String {
    clean_path(relative.as_ref()).to_string_lossy().into_owned()
}

/// Resolves a configured path while preserving explicit absolute and `~/` paths.
pub fn resolve_path(configured: impl AsRef<Path>) -> PathBuf {
    let compose_dir = std::env::var_os(COMPOSE_DIR_ENV);
    let current_dir = std::env::current_dir().ok();
    resolve_path_with(
        configured.as_ref(),
        compose_dir.as_deref(),
        current_dir.as_deref(),
        home_dir().as_deref(),
    )
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

/// Builds a path under `III_COMPOSE_DIR`, or under the process directory when
/// the worker runs outside Compose.
pub fn project_path(relative: impl AsRef<Path>) -> PathBuf {
    let compose_dir = std::env::var_os(COMPOSE_DIR_ENV);
    let current_dir = std::env::current_dir().ok();
    project_path_with(
        relative.as_ref(),
        compose_dir.as_deref(),
        current_dir.as_deref(),
    )
}

fn resolve_path_with(
    configured: &Path,
    compose_dir: Option<&OsStr>,
    current_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> PathBuf {
    if configured.is_absolute() {
        return configured.to_path_buf();
    }

    if configured == Path::new("~") {
        return home_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| configured.to_path_buf());
    }

    if let Ok(relative_to_home) = configured.strip_prefix("~") {
        if let Some(home_dir) = home_dir {
            return home_dir.join(relative_to_home);
        }
    }

    project_path_with(configured, compose_dir, current_dir)
}

fn project_path_with(
    relative: &Path,
    compose_dir: Option<&OsStr>,
    current_dir: Option<&Path>,
) -> PathBuf {
    let relative = clean_path(relative);
    let root = compose_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| current_dir.map(Path::to_path_buf));
    match root {
        Some(root) => root.join(relative),
        None => relative,
    }
}

fn clean_path(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_uses_compose_directory_when_available() {
        let resolved = resolve_path_with(
            Path::new("data/session-manager"),
            Some(OsStr::new("/workspace/project")),
            Some(Path::new("/worker/source")),
            Some(Path::new("/home/user")),
        );

        assert_eq!(
            resolved,
            PathBuf::from("/workspace/project/data/session-manager")
        );
    }

    #[test]
    fn relative_path_removes_current_directory_components() {
        let resolved = resolve_path_with(
            Path::new("./data/session-manager"),
            Some(OsStr::new("/workspace/project")),
            Some(Path::new("/worker/source")),
            Some(Path::new("/home/user")),
        );

        assert_eq!(
            resolved,
            PathBuf::from("/workspace/project/data/session-manager")
        );
    }

    #[test]
    fn default_path_stays_portable() {
        let default = default_path("data/session-manager");

        assert_eq!(default, "data/session-manager");
    }

    #[test]
    fn default_path_removes_current_directory_components() {
        let default = default_path("./data/session-manager");

        assert_eq!(default, "data/session-manager");
    }

    #[test]
    fn relative_path_uses_current_directory_outside_compose() {
        let resolved = resolve_path_with(
            Path::new("data/session-manager"),
            None,
            Some(Path::new("/worker/source")),
            Some(Path::new("/home/user")),
        );

        assert_eq!(
            resolved,
            PathBuf::from("/worker/source/data/session-manager")
        );
    }

    #[test]
    fn absolute_path_is_unchanged() {
        let resolved = resolve_path_with(
            Path::new("/var/lib/iii/sessions"),
            Some(OsStr::new("/workspace/project")),
            Some(Path::new("/worker/source")),
            Some(Path::new("/home/user")),
        );

        assert_eq!(resolved, PathBuf::from("/var/lib/iii/sessions"));
    }

    #[test]
    fn explicit_home_path_uses_home_directory() {
        let resolved = resolve_path_with(
            Path::new("~/.iii/sessions"),
            Some(OsStr::new("/workspace/project")),
            Some(Path::new("/worker/source")),
            Some(Path::new("/home/user")),
        );

        assert_eq!(resolved, PathBuf::from("/home/user/.iii/sessions"));
    }

    #[test]
    fn missing_bases_leave_relative_path_unchanged() {
        let resolved = resolve_path_with(Path::new("data/sessions"), None, None, None);

        assert_eq!(resolved, PathBuf::from("data/sessions"));
    }
}
