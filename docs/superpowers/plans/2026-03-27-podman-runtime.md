# Podman Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Docker with Podman as the sole container runtime, remove SandboxAdapter and `--isolation` flag, simplify dev mode to microsandbox-only.

**Architecture:** Swap `DockerAdapter` for `PodmanAdapter` (CLI binary rename + macOS machine management). Delete `SandboxAdapter`. Simplify dev cascade and CLI args. All changes in `engine/src/cli/`.

**Tech Stack:** Rust, tokio, clap, async-trait, podman CLI

**Spec:** `docs/superpowers/specs/2026-03-27-podman-runtime-design.md`

---

### Task 1: Create PodmanAdapter with machine management

**Files:**
- Create: `engine/src/cli/worker_manager/podman.rs`

This task creates the Podman adapter by adapting the Docker adapter. All `Command::new("docker")` calls become `Command::new("podman")`. Adds `podman_available()` probe and macOS `podman machine` auto-management.

- [ ] **Step 1: Create `podman.rs` with PodmanAdapter struct and `run_cmd` helper**

```rust
// engine/src/cli/worker_manager/podman.rs

use anyhow::{anyhow, Context, Result};
use std::io::Read;
use tokio::process::Command;

use super::adapter::{ContainerSpec, ContainerStatus, ImageInfo, RuntimeAdapter};

pub struct PodmanAdapter;

impl PodmanAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn run_cmd(args: &[&str]) -> Result<String> {
        let output = Command::new("podman")
            .args(args)
            .output()
            .await
            .with_context(|| format!("failed to execute: podman {}", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "podman {} failed (exit {}): {}",
                args.join(" "),
                output.status.code().unwrap_or(-1),
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}
```

- [ ] **Step 2: Implement RuntimeAdapter trait for PodmanAdapter**

```rust
#[async_trait::async_trait]
impl RuntimeAdapter for PodmanAdapter {
    async fn pull(&self, image: &str) -> Result<ImageInfo> {
        let local_check = Self::run_cmd(&["image", "inspect", image]).await;
        if local_check.is_ok() {
            tracing::info!(image = %image, "image found locally, skipping pull");
        } else {
            tracing::info!(image = %image, "pulling image from registry");
            Self::run_cmd(&["pull", image]).await?;
        }

        let size_str = Self::run_cmd(&["inspect", "--format", "{{.Size}}", image]).await;
        let size_bytes = size_str.ok().and_then(|s| s.parse::<u64>().ok());

        Ok(ImageInfo {
            image: image.to_string(),
            size_bytes,
        })
    }

    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>> {
        let container_name = format!("iii-extract-{}", uuid::Uuid::new_v4());
        Self::run_cmd(&["create", "--name", &container_name, image, "true"]).await?;

        let src = format!("{}:{}", container_name, path);
        let result = Self::run_cmd(&["cp", &src, "-"]).await;

        let _ = Self::run_cmd(&["rm", "-f", &container_name]).await;

        let tar_bytes = result.map(|s| s.into_bytes())?;

        let mut archive = tar::Archive::new(tar_bytes.as_slice());
        for entry in archive.entries()? {
            let mut entry = entry?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }

        Err(anyhow!("no files found in tar archive from podman cp"))
    }

    async fn start(&self, spec: &ContainerSpec) -> Result<String> {
        let mut args: Vec<String> = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            spec.name.clone(),
        ];

        for (key, val) in &spec.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, val));
        }

        if let Some(ref mem) = spec.memory_limit {
            args.push("--memory".to_string());
            args.push(k8s_mem_to_podman(mem));
        }

        if let Some(ref cpu) = spec.cpu_limit {
            args.push("--cpus".to_string());
            args.push(cpu.clone());
        }

        args.push(spec.image.clone());

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let container_id = Self::run_cmd(&args_ref).await?;

        tracing::info!(
            name = %spec.name,
            container_id = %container_id,
            "started container"
        );

        Ok(container_id)
    }

    async fn stop(&self, container_id: &str, timeout_secs: u32) -> Result<()> {
        tracing::info!(container_id = %container_id, timeout_secs = timeout_secs, "stopping container");
        let timeout_str = timeout_secs.to_string();
        Self::run_cmd(&["stop", "--time", &timeout_str, container_id]).await?;
        Ok(())
    }

    async fn status(&self, container_id: &str) -> Result<ContainerStatus> {
        let running_str = Self::run_cmd(&[
            "inspect", "--format", "{{.State.Running}}", container_id,
        ]).await?;

        let exit_code_str = Self::run_cmd(&[
            "inspect", "--format", "{{.State.ExitCode}}", container_id,
        ]).await?;

        let name_str = Self::run_cmd(&[
            "inspect", "--format", "{{.Name}}", container_id,
        ]).await?;

        Ok(ContainerStatus {
            name: name_str.trim_start_matches('/').to_string(),
            container_id: container_id.to_string(),
            running: running_str == "true",
            exit_code: exit_code_str.parse::<i32>().ok(),
        })
    }

    async fn logs(&self, container_id: &str, _follow: bool) -> Result<String> {
        Self::run_cmd(&["logs", "--tail", "100", container_id]).await
    }

    async fn remove(&self, container_id: &str) -> Result<()> {
        tracing::info!(container_id = %container_id, "removing container");
        Self::run_cmd(&["rm", "-f", container_id]).await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Add `k8s_mem_to_podman` helper, `podman_available()`, and `ensure_machine()` functions**

```rust
/// Convert Kubernetes-style memory strings (e.g. "256Mi", "1Gi") to Podman format ("256m", "1g").
pub fn k8s_mem_to_podman(value: &str) -> String {
    if let Some(n) = value.strip_suffix("Mi") {
        format!("{}m", n)
    } else if let Some(n) = value.strip_suffix("Gi") {
        format!("{}g", n)
    } else if let Some(n) = value.strip_suffix("Ki") {
        format!("{}k", n)
    } else {
        value.to_string()
    }
}

/// Check if Podman CLI is available.
pub async fn podman_available() -> bool {
    Command::new("podman")
        .args(["--version"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensure the Podman machine is running (macOS only).
///
/// On macOS, Podman requires a Linux VM. This function:
/// 1. Checks if a machine exists and its state via `podman machine inspect`
/// 2. If no machine → `podman machine init` + `podman machine start`
/// 3. If stopped → `podman machine start`
/// 4. If running → no-op
pub async fn ensure_machine() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }

    let inspect = Command::new("podman")
        .args(["machine", "inspect"])
        .output()
        .await;

    match inspect {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // podman machine inspect returns JSON; check State field
            if stdout.contains("\"Running\"") {
                tracing::info!("Podman machine already running");
                return Ok(());
            }
            // Machine exists but not running — start it
            tracing::info!("Starting Podman machine...");
            let start = Command::new("podman")
                .args(["machine", "start"])
                .output()
                .await
                .with_context(|| "failed to start podman machine")?;
            if !start.status.success() {
                let stderr = String::from_utf8_lossy(&start.stderr);
                return Err(anyhow!("podman machine start failed: {}", stderr.trim()));
            }
            tracing::info!("Podman machine started");
            Ok(())
        }
        _ => {
            // No machine exists — init and start
            tracing::info!("Initializing Podman machine...");
            let init = Command::new("podman")
                .args(["machine", "init"])
                .output()
                .await
                .with_context(|| "failed to init podman machine")?;
            if !init.status.success() {
                let stderr = String::from_utf8_lossy(&init.stderr);
                return Err(anyhow!("podman machine init failed: {}", stderr.trim()));
            }

            tracing::info!("Starting Podman machine...");
            let start = Command::new("podman")
                .args(["machine", "start"])
                .output()
                .await
                .with_context(|| "failed to start podman machine")?;
            if !start.status.success() {
                let stderr = String::from_utf8_lossy(&start.stderr);
                return Err(anyhow!("podman machine start failed: {}", stderr.trim()));
            }
            tracing::info!("Podman machine initialized and started");
            Ok(())
        }
    }
}
```

- [ ] **Step 4: Verify the file compiles**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii 2>&1 | head -30`

This will fail because `mod.rs` still references `docker` and `sandbox`. That's expected — we fix it in Task 2.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/worker_manager/podman.rs
git commit -m "feat(worker-manager): add PodmanAdapter with machine management"
```

---

### Task 2: Remove DockerAdapter and SandboxAdapter, update factory

**Files:**
- Delete: `engine/src/cli/worker_manager/docker.rs`
- Delete: `engine/src/cli/worker_manager/sandbox.rs`
- Modify: `engine/src/cli/worker_manager/mod.rs`

- [ ] **Step 1: Delete `docker.rs` and `sandbox.rs`**

```bash
cd /Users/andersonleal/projetos/motia/motia
rm engine/src/cli/worker_manager/docker.rs
rm engine/src/cli/worker_manager/sandbox.rs
```

- [ ] **Step 2: Update `mod.rs` — replace docker/sandbox with podman**

Replace the full contents of `engine/src/cli/worker_manager/mod.rs` with:

```rust
// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at support@motia.dev
// See LICENSE and PATENTS files for details.

pub mod adapter;
pub mod config;
pub mod health;
pub mod install;
pub mod manifest;
pub mod microsandbox;
pub mod podman;
pub mod registry;
pub mod spec;
pub mod state;
pub mod storage;
pub mod uninstall;

use std::sync::Arc;

use self::adapter::RuntimeAdapter;
use self::microsandbox::MicrosandboxAdapter;
use self::podman::PodmanAdapter;

/// Create a runtime adapter by name.
///
/// Supported runtimes:
/// - `"podman"` — Podman via CLI (default)
/// - `"microsandbox"` — Microsandbox via `msb` CLI
pub fn create_adapter(runtime: &str) -> Arc<dyn RuntimeAdapter> {
    match runtime {
        "microsandbox" => Arc::new(MicrosandboxAdapter::new()),
        _ => Arc::new(PodmanAdapter::new()),
    }
}
```

- [ ] **Step 3: Verify the module structure compiles**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii 2>&1 | head -40`

This will show errors in `microsandbox.rs` (still imports `DockerAdapter`) and `managed.rs` (still references sandbox). Expected — fixed in Tasks 3 and 4.

- [ ] **Step 4: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add -A engine/src/cli/worker_manager/
git commit -m "refactor(worker-manager): remove Docker and Sandbox adapters, add Podman factory"
```

---

### Task 3: Update MicrosandboxAdapter to delegate to Podman

**Files:**
- Modify: `engine/src/cli/worker_manager/microsandbox.rs:16` (import line)
- Modify: `engine/src/cli/worker_manager/microsandbox.rs:95-103` (pull/extract_file methods)

- [ ] **Step 1: Change the import from DockerAdapter to PodmanAdapter**

In `engine/src/cli/worker_manager/microsandbox.rs`, replace:

```rust
use super::docker::DockerAdapter;
```

with:

```rust
use super::podman::PodmanAdapter;
```

- [ ] **Step 2: Update `pull()` to delegate to PodmanAdapter**

In the `RuntimeAdapter` impl, replace:

```rust
    async fn pull(&self, image: &str) -> Result<ImageInfo> {
        // OCI images are runtime-agnostic — pull via Docker on the host
        DockerAdapter::new().pull(image).await
    }
```

with:

```rust
    async fn pull(&self, image: &str) -> Result<ImageInfo> {
        // OCI images are runtime-agnostic — pull via Podman on the host
        PodmanAdapter::new().pull(image).await
    }
```

- [ ] **Step 3: Update `extract_file()` to delegate to PodmanAdapter**

In the `RuntimeAdapter` impl, replace:

```rust
    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>> {
        // OCI images are runtime-agnostic — extract via Docker on the host
        DockerAdapter::new().extract_file(image, path).await
    }
```

with:

```rust
    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>> {
        // OCI images are runtime-agnostic — extract via Podman on the host
        PodmanAdapter::new().extract_file(image, path).await
    }
```

- [ ] **Step 4: Verify microsandbox.rs compiles**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii 2>&1 | head -20`

Still expect errors from `managed.rs` and `main.rs` — fixed in Tasks 4 and 5.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/worker_manager/microsandbox.rs
git commit -m "refactor(microsandbox): delegate pull/extract to PodmanAdapter"
```

---

### Task 4: Update managed.rs — remove sandbox, simplify dev, add early exit guard

**Files:**
- Modify: `engine/src/cli/managed.rs`

This is the largest change. We need to:
1. Remove `run_dev_sandbox()` function entirely
2. Simplify `handle_worker_dev()` to microsandbox-only
3. Update `engine_url_for_runtime()` — remove sandbox case, change docker to podman
4. Update `docker_reachable_host()` / `docker_engine_url()` — rename for podman
5. Add early exit guard to `start_managed_workers()`
6. Remove sandbox availability probes
7. Update tests

- [ ] **Step 1: Rename `docker_reachable_host` to `podman_reachable_host` and update for Podman**

In `engine/src/cli/managed.rs`, replace:

```rust
/// Translate bind addresses that are unreachable from inside Docker containers.
/// `0.0.0.0` and `localhost`/`127.0.0.1` refer to the container itself, not the host.
/// On macOS/Windows, Docker Desktop provides `host.docker.internal`.
/// On Linux, the host gateway IP is used (typically 172.17.0.1).
fn docker_reachable_host(address: &str) -> String {
    match address {
        "0.0.0.0" | "localhost" | "127.0.0.1" => {
            if cfg!(target_os = "linux") {
                // On Linux without Docker Desktop, use the default bridge gateway
                "172.17.0.1".to_string()
            } else {
                "host.docker.internal".to_string()
            }
        }
        other => other.to_string(),
    }
}
```

with:

```rust
/// Translate bind addresses that are unreachable from inside Podman containers.
/// `0.0.0.0` and `localhost`/`127.0.0.1` refer to the container itself, not the host.
/// Podman provides `host.containers.internal` on all platforms.
/// On Linux, the host gateway IP is used as fallback (typically 172.17.0.1).
fn podman_reachable_host(address: &str) -> String {
    match address {
        "0.0.0.0" | "localhost" | "127.0.0.1" => {
            if cfg!(target_os = "linux") {
                "host.containers.internal".to_string()
            } else {
                "host.containers.internal".to_string()
            }
        }
        other => other.to_string(),
    }
}
```

- [ ] **Step 2: Rename `docker_engine_url` to `podman_engine_url` and update `engine_url`**

Replace:

```rust
/// Build engine WebSocket URL from a bind address like "0.0.0.0:49134",
/// translating to a Docker-reachable host.
pub fn docker_engine_url(bind_addr: &str) -> String {
    let (host, port) = match bind_addr.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => (bind_addr, "49134"),
    };
    format!("ws://{}:{}", docker_reachable_host(host), port)
}

fn engine_url(address: &str, port: u16) -> String {
    format!("ws://{}:{}", docker_reachable_host(address), port)
}
```

with:

```rust
/// Build engine WebSocket URL from a bind address like "0.0.0.0:49134",
/// translating to a Podman-reachable host.
pub fn podman_engine_url(bind_addr: &str) -> String {
    let (host, port) = match bind_addr.rsplit_once(':') {
        Some((h, p)) => (h, p),
        None => (bind_addr, "49134"),
    };
    format!("ws://{}:{}", podman_reachable_host(host), port)
}

fn engine_url(address: &str, port: u16) -> String {
    format!("ws://{}:{}", podman_reachable_host(address), port)
}
```

- [ ] **Step 3: Simplify `engine_url_for_runtime` — remove sandbox case, rename docker to podman**

Replace:

```rust
/// Build engine WebSocket URL appropriate for the given runtime.
fn engine_url_for_runtime(runtime: &str, address: &str, port: u16, lan_ip: &Option<String>) -> String {
    match runtime {
        "microsandbox" => {
            // Microsandbox VMs have their own network namespace —
            // localhost inside the VM refers to the VM, not the host.
            // Use LAN IP so the worker can reach the engine on the host.
            let host = lan_ip.as_deref().unwrap_or(address);
            format!("ws://{}:{}", host, port)
        }
        "sandbox" => {
            // Docker Sandbox needs LAN IP to bypass MITM proxy
            let host = lan_ip.as_deref().unwrap_or(address);
            format!("ws://{}:{}", host, port)
        }
        _ => {
            // Standard Docker
            engine_url(address, port)
        }
    }
}
```

with:

```rust
/// Build engine WebSocket URL appropriate for the given runtime.
fn engine_url_for_runtime(runtime: &str, address: &str, port: u16, lan_ip: &Option<String>) -> String {
    match runtime {
        "microsandbox" => {
            // Microsandbox VMs have their own network namespace —
            // localhost inside the VM refers to the VM, not the host.
            // Use LAN IP so the worker can reach the engine on the host.
            let host = lan_ip.as_deref().unwrap_or(address);
            format!("ws://{}:{}", host, port)
        }
        _ => {
            // Standard Podman — uses host.containers.internal
            engine_url(address, port)
        }
    }
}
```

- [ ] **Step 4: Simplify `handle_worker_dev` to microsandbox-only**

Replace the entire `handle_worker_dev` function (lines 721-793):

```rust
/// `iii worker dev <path>` — run a worker project inside a microsandbox.
///
/// Requires microsandbox (`msb` CLI + running server).
pub async fn handle_worker_dev(
    path: &str,
    name: Option<&str>,
    address: &str,
    port: u16,
) -> i32 {
    // 1. Resolve absolute path
    let project_path = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} Invalid path '{}': {}", "error:".red(), path, e);
            return 1;
        }
    };

    // 2. Check microsandbox availability
    if !super::worker_manager::microsandbox::msb_available().await {
        eprintln!(
            "{} microsandbox is required for dev mode. Install msb and start the server.",
            "error:".red()
        );
        return 1;
    }

    eprintln!("  Runtime: {}", "microsandbox".bold());

    // 3. Load project info (from iii.worker.yaml or auto-detect)
    let project = match load_project_info(&project_path) {
        Some(p) => p,
        None => {
            eprintln!(
                "{} Could not detect project type in '{}'. Add iii.worker.yaml or use package.json/Cargo.toml/pyproject.toml.",
                "error:".red(),
                project_path.display()
            );
            return 1;
        }
    };

    let has_manifest = project_path.join(WORKER_MANIFEST).exists();
    if has_manifest {
        eprintln!("  {} loaded from {}", "✓".green(), WORKER_MANIFEST.bold());
    }

    let dir_name = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("worker");
    let sb_name = name.map(|n| n.to_string()).unwrap_or_else(|| format!("iii-dev-{}", dir_name));
    let project_str = project_path.to_string_lossy();

    // 4. Detect LAN IP and compute engine URL
    let lan_ip = detect_lan_ip().await;
    let engine_url = engine_url_for_runtime("microsandbox", address, port, &lan_ip);

    eprintln!("  {} project detected: {}", "✓".green(), project.name.bold());
    eprintln!("  Sandbox: {}", sb_name.bold());
    eprintln!("  Path: {}", project_str.dimmed());
    eprintln!("  Engine: {}", engine_url.bold());
    eprintln!();

    run_dev_microsandbox(&sb_name, &project_str, &project, &engine_url).await
}
```

- [ ] **Step 5: Delete `run_dev_sandbox` function entirely**

Remove the entire `run_dev_sandbox` function (lines 796-928 in the current file). This is the function that starts with:

```rust
/// Run a worker dev session inside a Docker Sandbox.
async fn run_dev_sandbox(sb_name: &str, project_str: &str, project: &ProjectInfo, engine_url: &str) -> i32 {
```

Delete it completely — it's no longer called.

- [ ] **Step 6: Update `start_managed_workers` — remove sandbox probe, add early exit guard**

Replace the entire `start_managed_workers` function with:

```rust
/// Start all managed workers and spawn a health check loop.
/// Called by the engine on startup so workers reconnect automatically.
pub async fn start_managed_workers(engine_url: &str) {
    let state = match LauncherState::load() {
        Ok(s) => s,
        Err(_) => return, // no state file — nothing to do
    };

    let workers_to_start: Vec<_> = state
        .managed_workers
        .iter()
        .filter(|(_, w)| w.status != "failed")
        .map(|(name, w)| (name.clone(), w.clone()))
        .collect();

    if workers_to_start.is_empty() {
        return;
    }

    // Only probe runtimes if there are workers to start
    let needs_podman = workers_to_start.iter().any(|(_, w)| w.runtime != "microsandbox");
    let needs_msb = workers_to_start.iter().any(|(_, w)| w.runtime == "microsandbox");

    if needs_podman {
        if super::worker_manager::podman::podman_available().await {
            tracing::info!("Podman runtime: available");
            // Ensure machine is running on macOS
            if let Err(e) = super::worker_manager::podman::ensure_machine().await {
                tracing::warn!("Failed to ensure Podman machine: {}", e);
            }
        } else {
            tracing::warn!("Podman runtime: unavailable — skipping podman workers");
        }
    }

    let has_msb = if needs_msb {
        let available = super::worker_manager::microsandbox::msb_available().await;
        if available {
            tracing::info!("Microsandbox runtime: available");
        } else {
            tracing::info!("Microsandbox runtime: unavailable");
        }
        available
    } else {
        false
    };

    tracing::info!(
        count = workers_to_start.len(),
        "Starting managed workers..."
    );

    for (name, worker) in &workers_to_start {
        if worker.runtime == "microsandbox" && !has_msb {
            tracing::warn!(
                worker = %name,
                "skipping microsandbox worker — msb server not available"
            );
            continue;
        }

        let adapter = super::worker_manager::create_adapter(&worker.runtime);
        let mut env = HashMap::new();
        env.insert("III_ENGINE_URL".to_string(), engine_url.to_string());
        env.insert(
            "III_AUTH_TOKEN".to_string(),
            worker.auth_token.clone().unwrap_or_default(),
        );
        let config_json =
            serde_json::to_string(&worker.config).unwrap_or_else(|_| "{}".to_string());
        env.insert(
            "III_WORKER_CONFIG".to_string(),
            data_encoding::BASE64.encode(config_json.as_bytes()),
        );

        let spec = ContainerSpec {
            name: name.clone(),
            image: worker.image.clone(),
            env,
            memory_limit: worker.memory_limit.clone(),
            cpu_limit: worker.cpu_limit.clone(),
        };

        // Clean up stale container
        let _ = adapter.stop(name, 5).await;
        let _ = adapter.remove(name).await;

        match adapter.start(&spec).await {
            Ok(new_id) => {
                tracing::info!(worker = %name, "Managed worker started");
                let mut fresh = LauncherState::load().unwrap_or_default();
                if let Some(w) = fresh.managed_workers.get_mut(name) {
                    w.container_id = new_id;
                    w.started_at = chrono::Utc::now();
                    w.status = "running".to_string();
                }
                let _ = fresh.save();
            }
            Err(e) => {
                tracing::warn!(worker = %name, error = %e, "Failed to start managed worker");
            }
        }
    }

    // Spawn background health check loop
    let url = engine_url.to_string();
    tokio::spawn(async move {
        super::worker_manager::health::run_health_loop(url).await;
    });
    tracing::info!("Health check loop started (every 15s)");
}
```

- [ ] **Step 7: Update tests in managed.rs**

Replace all the test functions at the bottom of the file. Remove sandbox-related tests, rename docker references to podman:

Remove these tests entirely:
- `engine_url_for_runtime_sandbox_uses_lan_ip`
- `engine_url_for_runtime_sandbox_falls_back_to_address`
- `engine_url_for_runtime_docker_uses_docker_host`

Replace `engine_url_translates_localhost_for_docker` with:

```rust
    #[test]
    fn engine_url_translates_localhost_for_podman() {
        let url = engine_url("localhost", 49134);
        assert_eq!(url, "ws://host.containers.internal:49134");
    }
```

Replace `engine_url_preserves_explicit_address`:

```rust
    #[test]
    fn engine_url_preserves_explicit_address() {
        assert_eq!(engine_url("10.0.0.1", 9999), "ws://10.0.0.1:9999");
    }
```

Replace `docker_engine_url_translates_bind_address` with:

```rust
    #[test]
    fn podman_engine_url_translates_bind_address() {
        let url = podman_engine_url("0.0.0.0:49134");
        assert_eq!(url, "ws://host.containers.internal:49134");
    }
```

Replace `docker_engine_url_preserves_explicit_host` with:

```rust
    #[test]
    fn podman_engine_url_preserves_explicit_host() {
        assert_eq!(
            podman_engine_url("192.168.1.5:49134"),
            "ws://192.168.1.5:49134"
        );
    }
```

Replace `engine_url_for_runtime_microsandbox_uses_lan_ip` (keep same logic):

```rust
    #[test]
    fn engine_url_for_runtime_microsandbox_uses_lan_ip() {
        let lan = Some("192.168.1.50".to_string());
        let url = engine_url_for_runtime("microsandbox", "0.0.0.0", 49134, &lan);
        assert_eq!(url, "ws://192.168.1.50:49134");
    }
```

Replace `engine_url_for_runtime_microsandbox_falls_back_to_address` (keep same logic):

```rust
    #[test]
    fn engine_url_for_runtime_microsandbox_falls_back_to_address() {
        let url = engine_url_for_runtime("microsandbox", "10.0.0.5", 9999, &None);
        assert_eq!(url, "ws://10.0.0.5:9999");
    }
```

Add new test for podman runtime URL:

```rust
    #[test]
    fn engine_url_for_runtime_podman_uses_podman_host() {
        let url = engine_url_for_runtime("podman", "0.0.0.0", 49134, &None);
        assert_eq!(url, "ws://host.containers.internal:49134");
    }
```

- [ ] **Step 8: Verify managed.rs compiles and tests pass**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii --lib managed::tests 2>&1 | tail -20`

Still expect errors in `main.rs` — fixed in Task 5.

- [ ] **Step 9: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/managed.rs
git commit -m "refactor(managed): remove sandbox dev flow, simplify to microsandbox-only dev, use Podman URLs"
```

---

### Task 5: Update main.rs — remove `--isolation`, update `--runtime`, fix references

**Files:**
- Modify: `engine/src/main.rs`

- [ ] **Step 1: Update `WorkerCommands::Add` — remove `isolation` field, update `--runtime` help**

Replace:

```rust
        /// Container runtime for managed workers ("docker", "sandbox", or "microsandbox")
        #[arg(long)]
        runtime: Option<String>,

        /// Isolation level — "standard" uses Docker, "strong" auto-selects
        /// microsandbox (preferred) or Docker Sandbox microVMs.
        /// Overrides --runtime when set.
        #[arg(long)]
        isolation: Option<String>,
```

with:

```rust
        /// Container runtime for managed workers ("podman" or "microsandbox")
        #[arg(long)]
        runtime: Option<String>,
```

- [ ] **Step 2: Update the `Add` match arm — remove isolation cascade, simplify**

Replace the entire `WorkerCommands::Add` handler (lines 319-372) with:

```rust
                WorkerCommands::Add {
                    worker_name,
                    force,
                    runtime,
                    address,
                    port,
                } => {
                    if let Some(rt) = runtime.as_deref() {
                        // Managed (OCI) worker
                        match worker_name.as_deref() {
                            Some(name) => {
                                cli::managed::handle_managed_add(name, rt, address, *port)
                                    .await
                            }
                            None => {
                                eprintln!(
                                    "{} Worker name or image reference is required with --runtime",
                                    "error:".red()
                                );
                                1
                            }
                        }
                    } else {
                        // Native worker install (existing behavior)
                        cli::handle_install(worker_name.as_deref(), *force).await
                    }
                }
```

- [ ] **Step 3: Update `run_serve` — rename `docker_engine_url` to `podman_engine_url`**

Replace both occurrences of `cli::managed::docker_engine_url(&addr)` with `cli::managed::podman_engine_url(&addr)`:

```rust
            .on_ready(|addr| async move {
                let url = cli::managed::podman_engine_url(&addr);
                cli::managed::start_managed_workers(&url).await;
            })
```

(Two occurrences — one in the `use_default_config` branch, one in the else branch.)

- [ ] **Step 4: Update `Dev` command help text**

Replace:

```rust
    /// Run a worker project inside a Docker Sandbox for isolated development.
    ///
    /// Auto-detects the project type (package.json, Cargo.toml, pyproject.toml)
    /// and runs it inside a microVM sandbox connected to the engine.
```

with:

```rust
    /// Run a worker project inside a microsandbox for isolated development.
    ///
    /// Auto-detects the project type (package.json, Cargo.toml, pyproject.toml)
    /// and runs it inside a microsandbox connected to the engine.
```

- [ ] **Step 5: Update tests — remove `--isolation` test, update `--runtime` test**

Remove the test `worker_add_parses_with_isolation_strong` entirely.

Update `worker_add_parses_with_runtime` — change "docker" to "podman":

```rust
    #[test]
    fn worker_add_parses_with_runtime() {
        let cli = Cli::try_parse_from([
            "iii", "worker", "add", "image-resize", "--runtime", "podman",
        ])
        .expect("should parse worker add with --runtime");
        match cli.command {
            Some(Commands::Worker(WorkerCommands::Add {
                worker_name,
                force,
                runtime,
                address,
                port,
            })) => {
                assert_eq!(worker_name.as_deref(), Some("image-resize"));
                assert!(!force);
                assert_eq!(runtime.as_deref(), Some("podman"));
                assert_eq!(address, "localhost");
                assert_eq!(port, DEFAULT_PORT);
            }
            _ => panic!("expected Worker Add subcommand"),
        }
    }
```

Update `worker_add_parses_with_worker_name` — remove `isolation` from the destructure pattern since the field no longer exists:

```rust
    #[test]
    fn worker_add_parses_with_worker_name() {
        let cli = Cli::try_parse_from(["iii", "worker", "add", "pdfkit@1.0.0"])
            .expect("should parse worker add with worker name");
        match cli.command {
            Some(Commands::Worker(WorkerCommands::Add {
                worker_name, force, runtime, ..
            })) => {
                assert_eq!(worker_name.as_deref(), Some("pdfkit@1.0.0"));
                assert!(!force);
                assert!(runtime.is_none());
            }
            _ => panic!("expected Worker Add subcommand"),
        }
    }
```

- [ ] **Step 6: Build and run all tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii 2>&1 | tail -30`

Expected: All tests pass. If there are compilation errors, fix them.

- [ ] **Step 7: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/main.rs
git commit -m "refactor(cli): remove --isolation flag, update --runtime to podman/microsandbox"
```

---

### Task 6: Full build verification

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii 2>&1`

Expected: All tests pass, zero warnings related to our changes.

- [ ] **Step 2: Run cargo clippy for lint check**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo clippy -p iii 2>&1 | tail -20`

Expected: No new warnings. Fix any clippy warnings introduced by our changes.

- [ ] **Step 3: Verify no dangling references to Docker or Sandbox**

Run: `cd /Users/andersonleal/projetos/motia/motia && grep -rn "DockerAdapter\|SandboxAdapter\|sandbox_available\|docker_engine_url\|docker_reachable_host\|\"docker\"" engine/src/ --include="*.rs" | grep -v "//.*docker\|target/" | head -20`

Expected: No matches (all references cleaned up). The only acceptable matches are in comments or string literals that describe the old system.

- [ ] **Step 4: Verify Podman adapter is correctly wired**

Run: `cd /Users/andersonleal/projetos/motia/motia && grep -rn "PodmanAdapter\|podman_available\|podman_engine_url\|podman_reachable_host" engine/src/ --include="*.rs" | head -20`

Expected: References in `podman.rs`, `mod.rs`, `microsandbox.rs`, `managed.rs`, and `main.rs`.
