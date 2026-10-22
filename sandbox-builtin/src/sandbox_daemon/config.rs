//! Fail-closed behavior:
//!   - `image_allowlist` empty denies every `sandbox::create` (see
//!     `catalog::check_allowlist`).
//!   - `parse_config` returns an error on a malformed config; the adapter
//!     exits non-zero instead of falling back to defaults.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PerImageCap {
    pub max_cpus: u32,
    pub max_memory_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_auto_install")]
    pub auto_install: bool,
    #[serde(default)]
    pub image_allowlist: Vec<String>,
    #[serde(default = "default_sandbox_idle_timeout")]
    pub default_idle_timeout_secs: u64,
    #[serde(default = "default_sandbox_max_concurrent")]
    pub max_concurrent_sandboxes: u32,
    /// Concurrent execs admitted per sandbox. Above this, `sandbox::exec`
    /// returns S003.
    ///
    /// Was effectively hardcoded to 1. Both ends of the exec channel already
    /// multiplex by `corr_id`, so serialization was policy, not a transport
    /// limit — but it cannot be unbounded either: the guest defaults to 1
    /// vCPU and 512 MB, so enough simultaneous interpreters OOM the VM. 4 is
    /// a floor that unblocks overlapping I/O-bound calls without letting a
    /// burst exhaust a small guest; raise it alongside `default_memory_mb`.
    ///
    /// `0` means ONE, not unlimited — the opposite of the usual convention
    /// for concurrency knobs, so it is warned about at startup. There is no
    /// "unlimited": every concurrent exec is a full interpreter in a guest
    /// that defaults to 512 MB.
    #[serde(default = "default_sandbox_max_concurrent_exec")]
    pub max_concurrent_exec_per_sandbox: u32,
    /// Hard ceiling on a single exec's deadline, in ms. A caller's
    /// `timeout_ms` is clamped to this.
    ///
    /// Needed because the reaper protects a sandbox with execs in flight from
    /// being reclaimed: without a ceiling on how long an exec may run, one
    /// caller-supplied `timeout_ms` could pin a VM indefinitely. The reaper's
    /// busy grace is derived from this value.
    #[serde(default = "default_sandbox_max_exec_timeout")]
    pub max_exec_timeout_ms: u64,
    #[serde(default = "default_sandbox_cpus")]
    pub default_cpus: u32,
    #[serde(default = "default_sandbox_memory")]
    pub default_memory_mb: u32,
    #[serde(default)]
    pub per_image_caps: std::collections::HashMap<String, PerImageCap>,
    /// Deployment-specific images beyond the built-in presets. Map key is
    /// the name used in `image_allowlist` and the `image` field on
    /// `sandbox::create`; value is a fully-qualified OCI reference
    /// (e.g. `ghcr.io/acme/my-app:1.2.3`).
    ///
    /// Preset names (`python`, `node`, `bash`, `alpine`) are reserved —
    /// declaring one here is rejected by `parse_config` so nothing can
    /// silently shadow the trusted catalog. An image named here still
    /// must appear in `image_allowlist` to be bootable.
    #[serde(default)]
    pub custom_images: std::collections::HashMap<String, String>,
}

fn default_sandbox_auto_install() -> bool {
    true
}
fn default_sandbox_idle_timeout() -> u64 {
    300
}
fn default_sandbox_max_concurrent() -> u32 {
    32
}
fn default_sandbox_max_concurrent_exec() -> u32 {
    crate::sandbox_daemon::registry::DEFAULT_MAX_EXEC_IN_FLIGHT
}
fn default_sandbox_max_exec_timeout() -> u64 {
    crate::sandbox_daemon::adapters::DEFAULT_EXEC_TIMEOUT_MS
}
fn default_sandbox_cpus() -> u32 {
    1
}
fn default_sandbox_memory() -> u32 {
    512
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            auto_install: default_sandbox_auto_install(),
            image_allowlist: Vec::new(),
            default_idle_timeout_secs: default_sandbox_idle_timeout(),
            max_concurrent_sandboxes: default_sandbox_max_concurrent(),
            max_concurrent_exec_per_sandbox: default_sandbox_max_concurrent_exec(),
            max_exec_timeout_ms: default_sandbox_max_exec_timeout(),
            default_cpus: default_sandbox_cpus(),
            default_memory_mb: default_sandbox_memory(),
            per_image_caps: std::collections::HashMap::new(),
            custom_images: std::collections::HashMap::new(),
        }
    }
}

/// Parse the value `configuration::get` returned. `None` (nothing stored)
/// gives the defaults. **Fail-closed**: any error propagates and the adapter
/// exits instead of falling back to defaults.
///
/// Rejects the legacy wrapped shape (`sandbox: {...}`) with a clear
/// migration message. That shape was the vm-worker wrapper around
/// `SandboxConfig` and went away with `vm::exec`.
pub fn parse_config(raw: Option<serde_json::Value>) -> Result<SandboxConfig> {
    let Some(raw) = raw else {
        return Ok(SandboxConfig::default());
    };
    reject_legacy_wrapped_shape(&raw)?;
    let cfg: SandboxConfig = serde_json::from_value(raw).context("parse sandbox-builtin config")?;
    validate_custom_images(&cfg).context("custom_images")?;
    Ok(cfg)
}

fn reject_legacy_wrapped_shape(raw: &serde_json::Value) -> Result<()> {
    if raw.get("sandbox").is_some() {
        anyhow::bail!(
            "legacy wrapped shape detected: move fields from `sandbox:` to the top level. \
             The vm-worker wrapper (`VmWorkerConfig`) and `vm::exec` were removed; \
             `SandboxConfig` is the top-level config now. \
             Example: `image_allowlist: [python, node]` at the top level, not under `sandbox:`."
        );
    }
    Ok(())
}

/// Reject `custom_images` entries that collide with a catalog preset
/// name. Shadowing is prevented at the `resolve_image` layer too, but
/// failing at load-time gives the operator a clear error instead of
/// letting a silently-inert key sit in their config.
fn validate_custom_images(sandbox: &SandboxConfig) -> Result<()> {
    use crate::sandbox_daemon::catalog;
    let presets: std::collections::HashSet<&str> = catalog::preset_names().collect();
    for key in sandbox.custom_images.keys() {
        if presets.contains(key.as_str()) {
            anyhow::bail!(
                "custom_images key '{key}' shadows a built-in preset — rename it \
                 (presets: {})",
                catalog::preset_names().collect::<Vec<_>>().join(", ")
            );
        }
        if key.is_empty() {
            anyhow::bail!("custom_images key must not be empty");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_shape() {
        let yaml = r#"
auto_install: true
image_allowlist:
  - python
  - node
default_idle_timeout_secs: 300
max_concurrent_sandboxes: 32
default_cpus: 1
default_memory_mb: 512
"#;
        let cfg: SandboxConfig = serde_yaml::from_str(yaml).expect("parse");
        assert!(cfg.auto_install);
        assert_eq!(
            cfg.image_allowlist,
            vec!["python", "node"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(cfg.default_idle_timeout_secs, 300);
        assert_eq!(cfg.max_concurrent_sandboxes, 32);
        assert_eq!(cfg.default_cpus, 1);
        assert_eq!(cfg.default_memory_mb, 512);
    }

    #[test]
    fn defaults_fill_in_when_fields_absent() {
        let cfg: SandboxConfig = serde_yaml::from_str("{}").expect("parse");
        assert!(cfg.auto_install);
        assert!(cfg.image_allowlist.is_empty());
        assert_eq!(cfg.default_idle_timeout_secs, 300);
        assert_eq!(cfg.max_concurrent_sandboxes, 32);
        assert_eq!(cfg.default_cpus, 1);
        assert_eq!(cfg.default_memory_mb, 512);
        assert!(cfg.per_image_caps.is_empty());
        assert!(cfg.custom_images.is_empty());
    }

    #[test]
    fn parses_custom_images_map() {
        let yaml = r#"
auto_install: true
image_allowlist: [my-app]
custom_images:
  my-app: ghcr.io/acme/my-app:1.2.3
  other: docker.io/tenant/other:latest
"#;
        let cfg: SandboxConfig = serde_yaml::from_str(yaml).expect("parse");
        assert_eq!(
            cfg.custom_images.get("my-app").unwrap(),
            "ghcr.io/acme/my-app:1.2.3"
        );
        assert_eq!(
            cfg.custom_images.get("other").unwrap(),
            "docker.io/tenant/other:latest"
        );
    }

    #[test]
    fn parses_per_image_caps() {
        let yaml = r#"
image_allowlist: [python]
per_image_caps:
  python: { max_cpus: 4, max_memory_mb: 2048 }
"#;
        let cfg: SandboxConfig = serde_yaml::from_str(yaml).expect("parse");
        let cap = cfg.per_image_caps.get("python").expect("python cap");
        assert_eq!(cap.max_cpus, 4);
        assert_eq!(cap.max_memory_mb, 2048);
    }

    #[test]
    fn parse_config_fills_defaults_and_fails_closed() {
        use serde_json::json;
        assert!(parse_config(None).unwrap().image_allowlist.is_empty());
        assert_eq!(
            parse_config(Some(json!({"image_allowlist": ["python", "node"]})))
                .unwrap()
                .image_allowlist,
            ["python", "node"]
        );
        assert!(parse_config(Some(json!({"sandbox": {}}))).is_err());
        assert!(parse_config(Some(json!({"custom_images": {"python": "ghcr.io/x/y:1"}}))).is_err());
    }

    #[test]
    fn parse_config_rejects_empty_custom_image_key() {
        let err = parse_config(Some(
            serde_json::json!({"custom_images": {"": "docker.io/whatever:latest"}}),
        ))
        .unwrap_err();
        assert!(format!("{err:#}").contains("must not be empty"));
    }
}
