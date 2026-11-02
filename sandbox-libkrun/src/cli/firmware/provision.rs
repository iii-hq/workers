//! Provisioning of libkrunfw and iii-init binaries.
//!
//! Both come from a local override or the compile-time embeds, and fail closed
//! (no download).

use std::path::PathBuf;

use super::constants::{LIBKRUNFW_VERSION, libkrunfw_filename};
use super::resolve::{resolve_init_binary, resolve_libkrunfw_dir};
use super::symlinks;

/// Ensure libkrunfw is available.
///
/// Resolution order:
/// 1. Check local resolution chain (env var, `~/.iii/lib/`, system paths)
/// 2. Extract from embedded bytes (compile-time `embed-libkrunfw` feature)
///
/// A copy in `~/.iii/lib/` that differs from the embedded bytes (another build's
/// firmware; the macOS name carries no version) is replaced by the embedded one.
///
/// Fails closed when neither has it; nothing is downloaded.
pub async fn ensure_libkrunfw() -> anyhow::Result<PathBuf> {
    ensure_libkrunfw_from(super::libkrunfw_bytes::LIBKRUNFW_BYTES).await
}

async fn ensure_libkrunfw_from(bytes: &[u8]) -> anyhow::Result<PathBuf> {
    // Concurrent first boots in one process would share the pid-named temp file.
    static PROVISIONING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _provisioning = PROVISIONING.lock().await;

    let home_lib = dirs::home_dir().map(|h| h.join(".iii").join("lib"));
    let filename = libkrunfw_filename();

    if let Some(dir) = resolve_libkrunfw_dir() {
        let ours = !bytes.is_empty() && Some(&dir) == home_lib.as_ref();
        if !ours || tokio::fs::read(dir.join(&filename)).await.ok().as_deref() == Some(bytes) {
            tracing::debug!(dir = %dir.display(), "libkrunfw found via resolution chain");
            return Ok(dir);
        }
        tracing::info!(dir = %dir.display(), "replacing libkrunfw extracted by another build");
    }

    if !bytes.is_empty() {
        let lib_dir = home_lib.ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
        tokio::fs::create_dir_all(&lib_dir).await?;

        let dest = lib_dir.join(&filename);
        // Write aside, then rename: a running VM keeps the old inode, and macOS kills a
        // process that loads a signed dylib rewritten in place.
        let tmp = lib_dir.join(format!(".{filename}.{}", std::process::id()));

        tracing::info!("extracting embedded libkrunfw to {}", dest.display());
        tokio::fs::write(&tmp, bytes).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        tokio::fs::rename(&tmp, &dest).await?;

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
/// 2. `III_INIT_PATH`
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

    // 2. III_INIT_PATH
    if let Some(path) = resolve_init_binary() {
        tracing::debug!(path = %path.display(), "iii-init found via resolution chain");
        eprintln!("  \u{2713} Found iii-init at {}", path.display());
        return Ok(path);
    }

    anyhow::bail!(
        "iii-init is not embedded in this build and III_INIT_PATH names no file; \
         run BUILD_TARGET=<host target triple> scripts/build-init.sh and rebuild \
         with --features embed-init"
    )
}

#[cfg(test)]
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

    #[cfg(unix)]
    #[test]
    fn ensure_libkrunfw_replaces_a_copy_extracted_by_another_build() {
        use std::os::unix::fs::MetadataExt;
        let _g = lock_home();
        with_clean_env(|| {
            let lib = dirs::home_dir().unwrap().join(".iii").join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            let file = lib.join(libkrunfw_filename());
            std::fs::write(&file, b"old firmware").unwrap();

            assert_eq!(
                block_on(ensure_libkrunfw_from(b"new firmware")).unwrap(),
                lib
            );
            assert_eq!(std::fs::read(&file).unwrap(), b"new firmware");
            // A matching copy is left alone.
            let ino = std::fs::metadata(&file).unwrap().ino();
            block_on(ensure_libkrunfw_from(b"new firmware")).unwrap();
            assert_eq!(std::fs::metadata(&file).unwrap().ino(), ino);
        });
    }

    /// Two first boots in one daemon provision at once; neither may fail or
    /// leave a temp file behind.
    #[cfg(unix)]
    #[test]
    fn concurrent_ensure_libkrunfw_calls_all_succeed() {
        let _g = lock_home();
        with_clean_env(|| {
            // Seeded, so a system-wide libkrunfw cannot satisfy the lookup instead.
            let lib = dirs::home_dir().unwrap().join(".iii").join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(lib.join(libkrunfw_filename()), b"old firmware").unwrap();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap();
            let results = rt.block_on(async {
                let calls: Vec<_> = (0..16)
                    .map(|_| tokio::spawn(ensure_libkrunfw_from(b"new firmware")))
                    .collect();
                futures::future::join_all(calls).await
            });
            for result in results {
                assert_eq!(result.unwrap().unwrap(), lib);
            }
            assert_eq!(
                std::fs::read(lib.join(libkrunfw_filename())).unwrap(),
                b"new firmware"
            );
            let leftovers: Vec<_> = std::fs::read_dir(&lib)
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|name| name.starts_with('.'))
                .collect();
            assert!(leftovers.is_empty(), "{leftovers:?}");
        });
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
