//! Provisioning of libkrunfw and iii-init binaries.
//!
//! Both follow the same two-stage resolution, and fail closed (no download):
//! 1. Check local filesystem (env var, `~/.iii/lib/`, adjacent to binary)
//! 2. Extract from embedded bytes (compile-time features)

use std::path::PathBuf;

use super::constants::{LIBKRUNFW_VERSION, libkrunfw_filename};
use super::resolve::{resolve_init_binary, resolve_libkrunfw_dir};
use super::symlinks;

/// Ensure libkrunfw is available.
///
/// Resolution order:
/// 1. Check local resolution chain (env var, `~/.iii/lib/`, adjacent to binary, system paths)
/// 2. Extract from embedded bytes (compile-time `embed-libkrunfw` feature)
///
/// Fails closed when neither has it; nothing is downloaded.
pub async fn ensure_libkrunfw() -> anyhow::Result<PathBuf> {
    if let Some(dir) = resolve_libkrunfw_dir() {
        tracing::debug!(dir = %dir.display(), "libkrunfw found via resolution chain");
        return Ok(dir);
    }

    let bytes = super::libkrunfw_bytes::LIBKRUNFW_BYTES;
    if !bytes.is_empty() {
        let lib_dir = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
            .join(".iii")
            .join("lib");
        tokio::fs::create_dir_all(&lib_dir).await?;

        let filename = libkrunfw_filename();
        let dest = lib_dir.join(&filename);

        tracing::info!("extracting embedded libkrunfw to {}", dest.display());
        tokio::fs::write(&dest, bytes).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
        }

        #[cfg(unix)]
        symlinks::create_libkrunfw_symlinks(&lib_dir);

        tracing::debug!(
            version = LIBKRUNFW_VERSION,
            path = %lib_dir.display(),
            "libkrunfw extracted"
        );

        return Ok(lib_dir);
    }

    anyhow::bail!(
        "libkrunfw is not embedded in this build and was not found locally; \
         rebuild with --features embed-libkrunfw"
    )
}

/// Ensure the iii-init binary is available.
///
/// Resolution order:
/// 1. Check if init is embedded in iii-filesystem (compile-time `embed-init` feature)
/// 2. Check local resolution chain (env var, `~/.iii/lib/iii-init`, adjacent to binary)
///
/// Fails closed when neither has it; nothing is downloaded.
///
/// Returns the path to the iii-init binary.
pub async fn ensure_init_binary() -> anyhow::Result<PathBuf> {
    // 1. Check if embedded init is available (non-empty INIT_BYTES)
    if iii_filesystem::init::has_init() {
        tracing::debug!("iii-init is embedded in this build");
        // When embedded, we don't need a file path — the filesystem serves it from memory.
        // Return a sentinel path to indicate "embedded" mode.
        return Ok(PathBuf::from("__embedded__"));
    }

    // 2. Check local resolution chain
    if let Some(path) = resolve_init_binary() {
        tracing::debug!(path = %path.display(), "iii-init found via resolution chain");
        eprintln!("  \u{2713} Found iii-init at {}", path.display());
        return Ok(path);
    }

    anyhow::bail!(
        "iii-init is not embedded in this build and was not found locally; \
         run BUILD_TARGET=<host target triple> scripts/build-init.sh and rebuild \
         with --features embed-init"
    )
}

// Both tests check what a build WITHOUT the embed does; with both embeds on this
// module would be empty and its helpers dead code under `clippy -D warnings`.
#[cfg(all(test, any(not(has_libkrunfw), not(feature = "embed-init"))))]
mod tests {
    use super::*;
    use crate::cli::test_support::lock_home;

    /// Run `f` with an empty `HOME` and both override vars unset, restoring them after.
    /// The caller must hold `lock_home()`.
    fn with_clean_env<T>(f: impl FnOnce() -> T) -> T {
        let home = tempfile::tempdir().unwrap();
        let keys = ["HOME", "III_LIBKRUNFW_PATH", "III_INIT_PATH"];
        let prior: Vec<_> = keys.iter().map(std::env::var_os).collect();
        // SAFETY: the caller holds TEST_HOME_LOCK, which serializes all env mutation.
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::remove_var("III_LIBKRUNFW_PATH");
            std::env::remove_var("III_INIT_PATH");
        }
        let out = f();
        for (k, v) in keys.iter().zip(prior) {
            match v {
                Some(v) => unsafe { std::env::set_var(k, v) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        out
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[cfg(not(has_libkrunfw))]
    #[test]
    fn ensure_libkrunfw_fails_closed_without_an_embed() {
        let _g = lock_home();
        let (found, result) = with_clean_env(|| {
            (
                resolve_libkrunfw_dir().is_some(),
                block_on(ensure_libkrunfw()),
            )
        });
        // A system-wide libkrunfw (e.g. Homebrew) legitimately satisfies the lookup.
        if !found {
            assert!(result.unwrap_err().to_string().contains("not embedded"));
        }
    }

    #[cfg(not(feature = "embed-init"))]
    #[test]
    fn ensure_init_binary_fails_closed_without_an_embed() {
        let _g = lock_home();
        let result = with_clean_env(|| block_on(ensure_init_binary()));
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not embedded") && err.contains("BUILD_TARGET="));
    }
}
