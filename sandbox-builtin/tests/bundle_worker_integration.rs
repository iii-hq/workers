// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Integration tests for the bundle manifest contract and resource caps that
//! `bundle_vm_command` re-applies before every boot.
//!
//! - Manifest contract: missing manifest, invalid YAML, name mismatch,
//!   scripts.setup smuggling, implausible runtime.base_image, size cap.
//! - Resource clamping: requests exceeding caps, size cap.

use std::path::Path;

use sandbox_builtin::cli::bundle_download::{
    MAX_BUNDLE_MANIFEST_BYTES, ResourceCaps, parse_bundle_resources, validate_bundle_manifest,
};
use sandbox_builtin::core::error::WorkerOpError;

fn write_manifest(dir: &Path, body: &str) {
    std::fs::write(dir.join("iii.worker.yaml"), body).unwrap();
}

// =============================================================================
// Section A: Manifest contract (validated through the public surface)
// =============================================================================

#[test]
fn manifest_missing_file_returns_typed_error() {
    let tmp = tempfile::tempdir().unwrap();
    let err = validate_bundle_manifest(tmp.path(), "foo").expect_err("missing manifest");
    let WorkerOpError::BundleManifestRejected { field, .. } = err else {
        panic!("expected BundleManifestRejected");
    };
    assert_eq!(field, "iii.worker.yaml");
}

#[test]
fn manifest_rejects_setup_and_implausible_base_image() {
    let cases = [
        (
            "name: foo\nscripts:\n  setup: \"x\"\n  start: \"node x.js\"\n",
            "scripts.setup",
        ),
        (
            "name: foo\nruntime:\n  base_image: \"bad image!\"\nscripts:\n  start: \"node x.js\"\n",
            "runtime.base_image",
        ),
    ];
    for (yaml, expected_field) in cases {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), yaml);
        let err = validate_bundle_manifest(tmp.path(), "foo").expect_err(expected_field);
        let WorkerOpError::BundleManifestRejected { field, .. } = err else {
            panic!("expected BundleManifestRejected for {expected_field}");
        };
        assert_eq!(field, expected_field);
    }
}

#[test]
fn manifest_accepts_install_and_custom_base_image() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(
        tmp.path(),
        "name: foo\nruntime:\n  base_image: oven/bun:1\nscripts:\n  install: \"x\"\n  start: \"node x.js\"\n",
    );
    let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("install + custom ref accepted");
    assert_eq!(cmd, "node x.js");
}

#[test]
fn manifest_invalid_yaml_returns_config_parse_error() {
    let tmp = tempfile::tempdir().unwrap();
    // Unbalanced YAML — { without matching } at the start of a key.
    write_manifest(tmp.path(), "name: {bad\nscripts:\n  start: x\n");
    let err = validate_bundle_manifest(tmp.path(), "foo").expect_err("yaml parse");
    assert!(
        matches!(err, WorkerOpError::ConfigParse { .. }),
        "expected ConfigParse, got {err:?}"
    );
}

#[test]
fn manifest_name_mismatch_returns_typed_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(tmp.path(), "name: bar\nscripts:\n  start: node x.js\n");
    let err = validate_bundle_manifest(tmp.path(), "foo").expect_err("name mismatch");
    let WorkerOpError::BundleManifestRejected { field, .. } = err else {
        panic!("expected BundleManifestRejected");
    };
    assert_eq!(field, "name");
}

#[test]
fn manifest_returns_start_command() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(tmp.path(), "name: foo\nscripts:\n  start: node bundle.js\n");
    let cmd = validate_bundle_manifest(tmp.path(), "foo").expect("happy");
    assert_eq!(cmd, "node bundle.js");
}

// =============================================================================
// Section B: Resource clamping
// =============================================================================

#[test]
fn resources_defaults_apply_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(tmp.path(), "name: foo\nscripts:\n  start: x\n");
    let r = parse_bundle_resources(tmp.path(), ResourceCaps::default()).unwrap();
    assert!(r.clamped_cpus.is_none());
    assert!(r.clamped_memory_mb.is_none());
}

#[test]
fn resources_clamped_cpu_request_reported() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(
        tmp.path(),
        "name: foo\nresources:\n  cpus: 32\n  memory: 1024\nscripts:\n  start: x\n",
    );
    let caps = ResourceCaps {
        max_cpus: 4,
        max_memory_mb: 4096,
    };
    let r = parse_bundle_resources(tmp.path(), caps).unwrap();
    assert_eq!(r.cpus, 4);
    assert_eq!(r.clamped_cpus, Some(32));
}

#[test]
fn resources_clamped_memory_request_reported() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(
        tmp.path(),
        "name: foo\nresources:\n  cpus: 1\n  memory: 999999\nscripts:\n  start: x\n",
    );
    let r = parse_bundle_resources(tmp.path(), ResourceCaps::default()).unwrap();
    assert_eq!(r.clamped_memory_mb, Some(999999));
}

// =============================================================================
// Section C: Manifest size cap
// =============================================================================

#[test]
fn validate_bundle_manifest_rejects_oversize_yaml() {
    // Billion-laughs defense: any iii.worker.yaml larger than
    // MAX_BUNDLE_MANIFEST_BYTES (64 KiB) must be refused BEFORE the
    // bytes reach serde_yaml::from_str — chained anchor/alias expansion
    // in serde_yaml 0.9.x can balloon a small file to gigabytes.
    let tmp = tempfile::tempdir().unwrap();
    let big = "# pad\n".repeat(((MAX_BUNDLE_MANIFEST_BYTES as usize) / 6) + 1);
    let body = format!("name: regress\nscripts:\n  start: \"node x.js\"\n{}", big);
    assert!(
        body.len() as u64 > MAX_BUNDLE_MANIFEST_BYTES,
        "test setup: oversize manifest must exceed cap"
    );
    write_manifest(tmp.path(), &body);

    let err = validate_bundle_manifest(tmp.path(), "regress")
        .expect_err("oversize manifest must be rejected");
    match err {
        WorkerOpError::BundleManifestRejected { field, reason } => {
            assert_eq!(field, "iii.worker.yaml");
            assert!(
                reason.contains("billion-laughs") || reason.contains("capped"),
                "reason should mention the cap: {reason}"
            );
        }
        other => panic!("expected BundleManifestRejected, got {other:?}"),
    }
}

#[test]
fn parse_bundle_resources_rejects_oversize_manifest() {
    // Same defense at the resource-parse seam (called by
    // bundle_vm_command on every boot). Even after install-time
    // validation, the manifest could be tampered with on disk before
    // the next start; the size cap fires before serde_yaml runs.
    let tmp = tempfile::tempdir().unwrap();
    let big = "# pad\n".repeat(((MAX_BUNDLE_MANIFEST_BYTES as usize) / 6) + 1);
    let body = format!("name: regress\nresources:\n  cpus: 2\n{}", big);
    write_manifest(tmp.path(), &body);

    let err = parse_bundle_resources(tmp.path(), ResourceCaps::default())
        .expect_err("oversize manifest must be rejected by parse_bundle_resources");
    assert!(
        matches!(err, WorkerOpError::BundleManifestRejected { .. }),
        "expected BundleManifestRejected, got {err:?}"
    );
}
