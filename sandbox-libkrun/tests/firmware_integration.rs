// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Integration tests for firmware resolution chain and constants.
//!
//! Covers requirements FW-01, FW-02 and FW-04:
//! - FW-01: Local firmware resolution (env var override, ~/.iii/lib/)
//! - FW-02: Embedded fallback behavior (documented, observational)
//! - FW-04: All sources fail returns None

mod common;

use common::assertions::assert_paths_eq;
use sandbox_libkrun::cli::firmware::constants::libkrunfw_filename;
use sandbox_libkrun::cli::firmware::resolve::{
    lib_path_env_var, resolve_init_binary, resolve_libkrunfw_dir,
};
use std::sync::Mutex;

/// Serializes tests that mutate environment variables (HOME, III_LIBKRUNFW_PATH, III_INIT_PATH).
/// Each integration test file runs in its own process, so only intra-file locking is needed.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Group 1: Firmware resolution with local path (FW-01)
// ---------------------------------------------------------------------------

/// FW-01: resolve_libkrunfw_dir finds firmware in ~/.iii/lib/ when present.
///
/// Threat mitigation T-03-05: HOME is saved before override and restored
/// immediately after the function call, before any assertions.
#[test]
fn resolve_libkrunfw_dir_finds_local_firmware() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let lib_dir = tmp.path().join(".iii").join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();

    let filename = libkrunfw_filename();
    std::fs::write(lib_dir.join(&filename), b"fake-firmware").unwrap();

    let original_home = std::env::var("HOME").ok();
    // SAFETY: test-only, serialized via ENV_LOCK
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("III_LIBKRUNFW_PATH");
    }

    let result = resolve_libkrunfw_dir();

    // Restore HOME immediately (T-03-05 mitigation)
    unsafe {
        if let Some(ref home) = original_home {
            std::env::set_var("HOME", home);
        }
    }

    assert!(
        result.is_some(),
        "expected Some when firmware exists in ~/.iii/lib/"
    );
    assert_paths_eq(&result.unwrap(), &lib_dir);
}

/// FW-01: resolve_libkrunfw_dir respects III_LIBKRUNFW_PATH env var override.
#[test]
fn resolve_libkrunfw_dir_env_var_override() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let filename = libkrunfw_filename();
    let file_path = tmp.path().join(&filename);
    std::fs::write(&file_path, b"fake-firmware").unwrap();

    let original_env = std::env::var("III_LIBKRUNFW_PATH").ok();
    unsafe {
        std::env::set_var("III_LIBKRUNFW_PATH", file_path.to_str().unwrap());
    }

    let result = resolve_libkrunfw_dir();

    unsafe {
        match original_env {
            Some(ref v) => std::env::set_var("III_LIBKRUNFW_PATH", v),
            None => std::env::remove_var("III_LIBKRUNFW_PATH"),
        }
    }

    assert!(
        result.is_some(),
        "expected Some with III_LIBKRUNFW_PATH override"
    );
    assert_paths_eq(&result.unwrap(), tmp.path());
}

// ---------------------------------------------------------------------------
// Group 2: Embedded fallback (FW-02)
// ---------------------------------------------------------------------------

/// FW-02: Documents the embedded firmware fallback behavior.
///
/// The `resolve_libkrunfw_dir` function does NOT directly check for embedded firmware --
/// that's handled by `ensure_libkrunfw` in the provision module. This test verifies
/// the behavior when no local firmware exists: the result depends on whether firmware
/// is present in system paths (/usr/lib, /usr/local/lib, Homebrew on macOS).
///
/// The full embedded fallback is tested at a higher level via `ensure_libkrunfw`
/// which is out of scope for this phase.
#[test]
fn resolve_libkrunfw_dir_embedded_fallback_conditional() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    let original_env = std::env::var("III_LIBKRUNFW_PATH").ok();
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("III_LIBKRUNFW_PATH");
    }

    let result = resolve_libkrunfw_dir();

    unsafe {
        if let Some(ref home) = original_home {
            std::env::set_var("HOME", home);
        }
        match original_env {
            Some(ref v) => std::env::set_var("III_LIBKRUNFW_PATH", v),
            None => std::env::remove_var("III_LIBKRUNFW_PATH"),
        }
    }

    // The result depends on the host system:
    // - Some(path): system paths have libkrunfw installed (e.g., /usr/lib, Homebrew)
    // - None: no system-level libkrunfw (typical in CI/dev without VM support)
    match result {
        Some(path) => {
            let path_str = path.display().to_string();
            assert!(
                path_str.starts_with("/usr/") || path_str.starts_with("/opt/"),
                "expected system path, got: {path_str}"
            );
        }
        None => {
            // Expected when no system-level libkrunfw is installed
        }
    }
}

// ---------------------------------------------------------------------------
// Group 3: All sources fail (FW-04)
// ---------------------------------------------------------------------------

/// FW-04: resolve_libkrunfw_dir returns None (or system path) when env var and HOME fail.
///
/// The function checks: env var (nonexistent file), ~/.iii/lib/ (empty tempdir),
/// system paths.
/// If system paths don't have libkrunfw (typical in CI/dev), result is None.
/// If system has it installed, result is Some pointing to a system path.
#[test]
fn resolve_libkrunfw_dir_returns_none_when_all_sources_fail() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    let original_env = std::env::var("III_LIBKRUNFW_PATH").ok();
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("III_LIBKRUNFW_PATH", "/nonexistent/libkrunfw");
    }

    let result = resolve_libkrunfw_dir();

    unsafe {
        if let Some(ref home) = original_home {
            std::env::set_var("HOME", home);
        }
        match original_env {
            Some(ref v) => std::env::set_var("III_LIBKRUNFW_PATH", v),
            None => std::env::remove_var("III_LIBKRUNFW_PATH"),
        }
    }

    // System path caveat: if system has libkrunfw installed, we get Some.
    match result {
        None => {} // Expected in most CI/dev environments
        Some(path) => {
            let path_str = path.display().to_string();
            assert!(
                path_str.starts_with("/usr/") || path_str.starts_with("/opt/"),
                "if found, should be a system path, got: {path_str}"
            );
        }
    }
}

/// FW-04: resolve_init_binary returns None when all sources fail.
#[test]
fn resolve_init_binary_returns_none_when_all_sources_fail() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    let original_env = std::env::var("III_INIT_PATH").ok();
    unsafe {
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("III_INIT_PATH", "/nonexistent/iii-init");
    }

    let result = resolve_init_binary();

    unsafe {
        if let Some(ref home) = original_home {
            std::env::set_var("HOME", home);
        }
        match original_env {
            Some(ref v) => std::env::set_var("III_INIT_PATH", v),
            None => std::env::remove_var("III_INIT_PATH"),
        }
    }

    assert!(
        result.is_none(),
        "expected None when all iii-init sources fail, got: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Group 4: Constants verification (supporting tests)
// ---------------------------------------------------------------------------

/// lib_path_env_var returns the correct platform-specific variable name.
#[test]
fn lib_path_env_var_is_platform_correct() {
    let var = lib_path_env_var();
    if cfg!(target_os = "macos") {
        assert_eq!(var, "DYLD_LIBRARY_PATH");
    } else {
        assert_eq!(var, "LD_LIBRARY_PATH");
    }
}

/// libkrunfw_filename returns the correct platform-specific library filename.
#[test]
fn libkrunfw_filename_is_platform_correct() {
    let name = libkrunfw_filename();
    assert!(
        name.contains("libkrunfw"),
        "filename should contain 'libkrunfw': {name}"
    );
    if cfg!(target_os = "macos") {
        assert!(
            name.ends_with(".dylib"),
            "macOS should end with '.dylib': {name}"
        );
    } else {
        // Linux: libkrunfw.so.5.6.2 -- ends with a version number
        assert!(name.contains(".so."), "Linux should contain '.so.': {name}");
    }
}
