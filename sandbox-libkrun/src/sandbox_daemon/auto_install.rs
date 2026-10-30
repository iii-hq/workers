use crate::cli::rootfs_cache;
use crate::sandbox_daemon::errors::SandboxError;
use crate::sandbox_daemon::events::SandboxCreateEvent;

/// Emits `PullingImage` and `Unpacking` events only when an actual
/// pull happens; a cache hit fires neither.
pub async fn auto_install_image(
    image_name: &str,
    oci_ref: &str,
    mut on_event: impl FnMut(SandboxCreateEvent) + Send + 'static,
) -> Result<std::path::PathBuf, SandboxError> {
    let image_ref = oci_ref.to_string();
    let dest = rootfs_cache::ensure_rootfs(oci_ref, move || {
        on_event(SandboxCreateEvent::PullingImage {
            image_ref: image_ref.clone(),
            progress_bytes: 0,
            total_bytes: None,
        });
        on_event(SandboxCreateEvent::Unpacking);
    })
    .await
    .map_err(|e| {
        SandboxError::auto_install_failed(
            image_name,
            format!("pull_and_extract_rootfs failed: {e}"),
        )
    })?;

    // Post-extract sanity: oci-client returns Ok(()) from pull even
    // when the manifest-list has no matching platform for the host
    // arch (layer_count = 0, silent no-op). Catch that here so the
    // caller sees S102 instead of an opaque S300 "shell socket didn't
    // appear" 30s later. (Redundant with `ensure_rootfs`'s own
    // `is_populated` guard on cache hits, but cheap and load-bearing
    // on fresh pulls.)
    if !dest.join("bin").exists() {
        return Err(SandboxError::auto_install_failed(
            image_name,
            format!(
                "rootfs extracted to {} but bin/ is missing -- likely no \
                 linux/{} manifest for {}",
                dest.display(),
                std::env::consts::ARCH,
                oci_ref,
            ),
        ));
    }

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_s102_on_invalid_image_ref() {
        let err = auto_install_image("__autoinstall-test", "::not a real ref::", |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S102");
    }
}
