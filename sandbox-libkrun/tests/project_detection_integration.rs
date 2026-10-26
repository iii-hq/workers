// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Integration tests for manifest loading and script inference (PROJ-05).

use sandbox_libkrun::cli::project::{WORKER_MANIFEST, infer_scripts, load_from_manifest};

// ---------------------------------------------------------------------------
// Group 1: infer_scripts tests (PROJ-05)
// ---------------------------------------------------------------------------

/// PROJ-05: Infer scripts for TypeScript/bun runtime.
#[test]
fn infer_scripts_typescript_bun() {
    let (setup, install, run) = infer_scripts("typescript", "bun", "src/index.ts");
    assert!(
        setup.contains("bun.sh/install"),
        "setup should contain 'bun.sh/install': {setup}"
    );
    assert!(
        install.contains("bun install"),
        "install should contain 'bun install': {install}"
    );
    assert!(
        run.contains("bun src/index.ts"),
        "run should contain 'bun src/index.ts': {run}"
    );
}

/// PROJ-05: Infer scripts for TypeScript/npm runtime.
#[test]
fn infer_scripts_typescript_npm() {
    let (_setup, install, run) = infer_scripts("typescript", "npm", "src/index.ts");
    assert!(
        install.contains("npm install"),
        "install should contain 'npm install': {install}"
    );
    assert!(
        run.contains("npx tsx src/index.ts"),
        "run should contain 'npx tsx src/index.ts': {run}"
    );
}

/// PROJ-05: Infer scripts for Python/pip runtime.
#[test]
fn infer_scripts_python_pip() {
    let (setup, install, run) = infer_scripts("python", "pip", "my_module");
    assert!(
        setup.contains("python3"),
        "setup should contain 'python3': {setup}"
    );
    assert!(
        install.contains(".venv/bin/pip"),
        "install should contain '.venv/bin/pip': {install}"
    );
    assert!(
        run.contains(".venv/bin/python -m my_module"),
        "run should contain '.venv/bin/python -m my_module': {run}"
    );
}

/// PROJ-05: Infer scripts for Rust/cargo runtime.
#[test]
fn infer_scripts_rust_cargo() {
    let (setup, install, run) = infer_scripts("rust", "cargo", "src/main.rs");
    assert!(
        setup.contains("rustup"),
        "setup should contain 'rustup': {setup}"
    );
    assert!(
        install.contains("cargo build"),
        "install should contain 'cargo build': {install}"
    );
    assert!(
        run.contains("cargo run"),
        "run should contain 'cargo run': {run}"
    );
}

/// PROJ-05: Unknown language/package manager returns entry as run command.
#[test]
fn infer_scripts_unknown_returns_entry() {
    let (setup, install, run) = infer_scripts("unknown", "unknown", "main.sh");
    assert!(setup.is_empty(), "setup should be empty: {setup}");
    assert!(install.is_empty(), "install should be empty: {install}");
    assert_eq!(run, "main.sh");
}

// ---------------------------------------------------------------------------
// Group 2: Manifest loading (PROJ-05 augmentation)
// ---------------------------------------------------------------------------

/// PROJ-05: load_from_manifest with explicit scripts and env filtering.
#[test]
fn load_from_manifest_with_explicit_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join(WORKER_MANIFEST);
    let yaml = r#"
name: my-worker
scripts:
  setup: "apt-get update"
  install: "npm install"
  start: "node server.js"
env:
  FOO: bar
  III_URL: skip
  III_ENGINE_URL: skip
"#;
    std::fs::write(&manifest_path, yaml).unwrap();

    let info = load_from_manifest(&manifest_path).unwrap();
    assert_eq!(info.name, "my-worker");
    assert_eq!(info.setup_cmd, "apt-get update");
    assert_eq!(info.install_cmd, "npm install");
    assert_eq!(info.run_cmd, "node server.js");
    assert_eq!(info.env.get("FOO").unwrap(), "bar");
    assert!(
        !info.env.contains_key("III_URL"),
        "III_URL should be filtered out"
    );
    assert!(
        !info.env.contains_key("III_ENGINE_URL"),
        "III_ENGINE_URL should be filtered out"
    );
}

/// PROJ-05: load_from_manifest infers scripts from runtime section when no explicit scripts.
#[test]
fn load_from_manifest_infers_scripts_from_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join(WORKER_MANIFEST);
    let yaml = r#"
name: my-bun-worker
runtime:
  kind: typescript
  package_manager: bun
  entry: src/index.ts
"#;
    std::fs::write(&manifest_path, yaml).unwrap();

    let info = load_from_manifest(&manifest_path).unwrap();
    assert_eq!(info.name, "my-bun-worker");
    assert!(
        info.setup_cmd.contains("bun.sh/install"),
        "setup_cmd should contain 'bun.sh/install': {}",
        info.setup_cmd
    );
    assert!(
        info.run_cmd.contains("bun src/index.ts"),
        "run_cmd should contain 'bun src/index.ts': {}",
        info.run_cmd
    );
}
