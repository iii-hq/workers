//! Runtime resolution of libkrunfw shared library and iii-init binary locations.
//!
//! libkrunfw is looked up in order at:
//! 1. `III_LIBKRUNFW_PATH` (file path -> parent dir, or directory)
//! 2. `~/.iii/lib/` (where the embedded copy is extracted)
//! 3. System paths (`/usr/lib`, `/usr/local/lib`, Homebrew on macOS)
//!
//! An on-disk iii-init is only taken from `III_INIT_PATH`.

use std::path::{Path, PathBuf};

use super::constants::libkrunfw_filename;

/// Returns the platform-correct environment variable name for the shared library search path.
///
/// - macOS: `DYLD_LIBRARY_PATH`
/// - Linux: `LD_LIBRARY_PATH`
pub fn lib_path_env_var() -> &'static str {
    if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

/// Resolve the directory containing libkrunfw.
///
/// Checks (in order): env var override, `~/.iii/lib/`, system paths.
/// Returns `None` if the library is not found in any location.
pub fn resolve_libkrunfw_dir() -> Option<PathBuf> {
    let filename = libkrunfw_filename();

    // Build list of paths to check
    let env_path = std::env::var("III_LIBKRUNFW_PATH").ok();
    let lib_dir = dirs::home_dir().map(|h| h.join(".iii").join("lib"));

    let system_paths: Vec<&Path> = if cfg!(target_os = "macos") {
        vec![
            Path::new("/usr/lib"),
            Path::new("/usr/local/lib"),
            Path::new("/opt/homebrew/opt/libkrun/lib"),
        ]
    } else {
        vec![Path::new("/usr/lib"), Path::new("/usr/local/lib")]
    };

    resolve_dir_with_paths(
        env_path.as_deref(),
        lib_dir.as_deref(),
        &system_paths,
        &filename,
    )
}

/// The on-disk iii-init named by `III_INIT_PATH`, if it is a file. Release
/// builds embed iii-init instead.
pub fn resolve_init_binary() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("III_INIT_PATH")?);
    p.is_file().then_some(p)
}

/// Internal testable resolution function.
fn resolve_dir_with_paths(
    env_path: Option<&str>,
    lib_dir: Option<&Path>,
    system_paths: &[&Path],
    filename: &str,
) -> Option<PathBuf> {
    // 1. III_LIBKRUNFW_PATH env var
    if let Some(env_val) = env_path {
        let p = PathBuf::from(env_val);
        if p.is_file() {
            return p.parent().map(|d| d.to_path_buf());
        }
        if p.is_dir() && p.join(filename).exists() {
            return Some(p);
        }
    }

    // 2. ~/.iii/lib/
    if let Some(dir) = lib_dir
        && dir.join(filename).exists()
    {
        return Some(dir.to_path_buf());
    }

    // 3. System paths
    for path in system_paths {
        if path.join(filename).exists() {
            return Some(path.to_path_buf());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lib_path_env_var() {
        let var = lib_path_env_var();
        if cfg!(target_os = "macos") {
            assert_eq!(var, "DYLD_LIBRARY_PATH");
        } else {
            assert_eq!(var, "LD_LIBRARY_PATH");
        }
    }

    #[test]
    fn test_resolve_env_var_file_path() {
        let tmp = TempDir::new().unwrap();
        let filename = libkrunfw_filename();
        let file_path = tmp.path().join(&filename);
        std::fs::write(&file_path, b"fake").unwrap();

        let result =
            resolve_dir_with_paths(Some(file_path.to_str().unwrap()), None, &[], &filename);
        assert_eq!(result, Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn test_resolve_not_found() {
        let result = resolve_dir_with_paths(None, None, &[], &libkrunfw_filename());
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_init_binary_env_var() {
        // Held like every other test that touches III_INIT_PATH, so they do not race.
        let _g = crate::cli::test_support::lock_home();
        let tmp = TempDir::new().unwrap();
        let init_path = tmp.path().join("iii-init");
        std::fs::write(&init_path, b"fake-init").unwrap();

        // SAFETY: the env lock above serializes access to the variable.
        unsafe {
            std::env::set_var("III_INIT_PATH", init_path.to_str().unwrap());
        }
        let result = resolve_init_binary();
        unsafe {
            std::env::remove_var("III_INIT_PATH");
        }

        assert_eq!(result, Some(init_path));
    }
}
