//! Shared read-only rootfs overlay: feature flag and init-capability
//! handshake.
//!
//! The overlay model (shared read-only erofs base + per-worker writable
//! upper, assembled by iii-init) is ON BY DEFAULT. It is gated by two checks:
//!
//! 1. Feature flag — `III_ROOTFS_MODE`. Anything other than `legacy`/`off`/
//!    `0`/`false` (including unset) means overlay. Operators opt back into the
//!    legacy per-worker-clone model with `III_ROOTFS_MODE=legacy`.
//!
//! 2. Capability handshake — overlay requires an EMBEDDED iii-init. An
//!    embedded init is compiled into *this* sandbox-libkrun binary, so it is
//!    guaranteed version-matched and overlay-capable. An on-disk init
//!    (`III_INIT_PATH`) may predate overlay support: it would ignore the
//!    `III_BLOCK_ROOT_*` env and boot the legacy pivot against a layout that
//!    does not exist, producing a broken VM. Embedded init can never be
//!    stale, so its presence is the safe capability signal.

/// Overlay is on by default; `III_ROOTFS_MODE=legacy|off|0|false|no`
/// disables it.
pub fn overlay_enabled() -> bool {
    std::env::var("III_ROOTFS_MODE").map_or(true, |v| mode_value_enables_overlay(&v))
}

/// Pure mapping of a mode string to "overlay on?". Anything other than the
/// recognized disable words (case-insensitive) enables overlay.
fn mode_value_enables_overlay(v: &str) -> bool {
    !matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "legacy" | "off" | "0" | "false" | "no"
    )
}

/// Decide whether to boot in overlay mode: flag on AND an overlay-capable
/// (embedded) iii-init is available — see the capability-handshake rationale in
/// the module docs. Emits a one-line reason when the flag is on but we must
/// fall back to legacy, so a non-embedded build (or a deliberate
/// `III_ROOTFS_MODE=legacy`) is visible in the worker log rather than silent.
pub fn overlay_active() -> bool {
    if !overlay_enabled() {
        return false;
    }
    if !iii_filesystem::init::has_init() {
        // Common+expected in non-embed (dev) builds, so keep it quiet — a
        // per-start stderr warning would spam every worker boot. Operators
        // who expect overlay can see this at debug level.
        tracing::debug!(
            "overlay rootfs requested but iii-init is not embedded; using legacy per-worker rootfs"
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_value_mapping_disables_on_legacy_words_only() {
        for v in ["legacy", "off", "0", "false", "no", "LEGACY", " Off "] {
            assert!(!mode_value_enables_overlay(v), "{v} should disable overlay");
        }
        for v in ["overlay", "on", "1", "true", "yes", "anything-else"] {
            assert!(mode_value_enables_overlay(v), "{v} should enable overlay");
        }
    }
}
