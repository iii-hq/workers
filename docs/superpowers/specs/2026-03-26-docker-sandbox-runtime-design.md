# Docker Sandbox Runtime — Design Spec

**Date:** 2026-03-26
**Status:** Approved
**Replaces:** Firecracker runtime adapter

## Summary

Replace the Firecracker runtime adapter with Docker Sandboxes as the "strong isolation" runtime. Docker Sandboxes provide hypervisor-level microVM isolation that works on macOS and Windows — no Linux/KVM requirement. The same OCI image runs in both standard Docker containers and sandboxed microVMs.

Two runtimes: `docker` (standard) and `sandbox` (strong isolation). Two deployment modes: per-worker sandboxes managed by the engine, or the entire engine running inside a sandbox.

## Design Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | Strong isolation runtime | Docker Sandbox (replaces Firecracker) | microVM isolation on Mac/Windows out of the box. Same security model as Firecracker, no KVM requirement. |
| 2 | Runtime count | Two: `docker` + `sandbox` | YAGNI. Firecracker, gVisor, Kata all need Linux. Docker Sandbox covers the cross-platform strong isolation case. |
| 3 | Auto-detection | Probe `docker sandbox ls` on engine boot | No errors if unavailable, no fallbacks. Sandbox runtime simply hidden when Docker Desktop isn't present. |
| 4 | Engine-in-sandbox | Supported, no code needed | User runs `docker sandbox run iii ~/project`. Inside the sandbox, the engine uses standard Docker (the sandbox's private daemon). Natural nesting. |
| 5 | Image operations | Delegated to Docker | `pull` and `extract_file` use `docker` CLI directly. OCI images are runtime-agnostic. Only lifecycle ops differ. |

## Runtime Model

| Runtime | Isolation | CLI Flag | Use Case |
|---------|-----------|----------|----------|
| `docker` | Linux namespaces (standard container) | `--runtime docker` (default) | Development, trusted workers |
| `sandbox` | microVM (hypervisor-level, private Docker daemon) | `--runtime sandbox` or `--isolation strong` | Untrusted workers, agent-driven execution |

### `--isolation` Mapping

- `standard` -> `docker`
- `strong` -> `sandbox`

## Two Deployment Modes

### Mode 1: Per-Worker Sandbox

```bash
iii worker add image-resize --runtime sandbox
```

Engine runs on host. Each worker gets its own sandbox microVM with a private Docker daemon. Worker can't see host containers, files, or other workers.

### Mode 2: Engine-in-Sandbox

```bash
docker sandbox run iii ~/project
```

Entire iii engine runs inside one sandbox. Workers run as regular Docker containers inside the sandbox's private Docker daemon. From the host's perspective: one sandbox, full iii deployment inside. No engine code changes needed — the engine just uses `--runtime docker` and the sandbox provides the isolation transparently.

## SandboxAdapter

Implements `RuntimeAdapter` trait. Replaces `FirecrackerAdapter`.

### Command Mapping

| Operation | DockerAdapter | SandboxAdapter |
|-----------|--------------|----------------|
| pull | `docker pull` | `docker pull` (same — OCI is runtime-agnostic) |
| extract_file | `docker create` + `docker cp` | Same as Docker |
| start | `docker run -d` | `docker sandbox create --name iii-<name>` then `docker sandbox exec iii-<name> docker run -d <image> <env>` |
| stop | `docker stop --time N` | `docker sandbox exec iii-<name> docker stop <container>` then `docker sandbox rm iii-<name>` |
| status | `docker inspect` | `docker sandbox ls` + parse output for sandbox running state |
| logs | `docker logs --tail 100` | `docker sandbox exec iii-<name> docker logs --tail 100 <container>` |
| remove | `docker rm -f` | `docker sandbox rm iii-<name>` |

### Start Flow (Per-Worker Sandbox)

1. `docker sandbox create --name iii-<worker-name>` — create the microVM
2. Wait for sandbox to be ready
3. `docker sandbox exec iii-<worker-name> docker pull <image>` — pull image inside sandbox
4. `docker sandbox exec iii-<worker-name> docker run -d --name <worker-name> -e III_ENGINE_URL=... -e III_AUTH_TOKEN=... -e III_WORKER_CONFIG=... <image>` — run worker container inside sandbox
5. Store sandbox name as `container_id` in state

### State

`ManagedWorker.runtime` stores `"sandbox"`. `ManagedWorker.container_id` stores the sandbox name (e.g. `iii-image-resize`).

## Auto-Detection

On engine boot, before starting managed workers:

```rust
async fn sandbox_available() -> bool {
    Command::new("docker")
        .args(["sandbox", "ls"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

- If available: log `Sandbox runtime: available (Docker Desktop 4.58+)`
- If unavailable: log `Sandbox runtime: unavailable`
- If a worker has `runtime: "sandbox"` but sandbox isn't available: log warning, skip it. Don't crash, don't fall back.

## Health Check

The existing health check loop already resolves the adapter per-worker via `create_adapter(&worker.runtime)`. No structural changes needed.

- `SandboxAdapter.status()` calls `docker sandbox ls`, parses output
- Same exponential backoff (5s -> 10s -> 20s -> 40s -> 60s cap)
- Same 5-retry max before marking as `"failed"`

### Restart for Sandbox Workers

1. `docker sandbox rm iii-<name>` (cleanup old sandbox)
2. `docker sandbox create --name iii-<name>` (fresh microVM)
3. `docker sandbox exec iii-<name> docker run -d ...` (start worker inside)

## CLI Changes

### New

```bash
iii worker add <image> --runtime sandbox
iii worker add <image> --isolation strong    # maps to sandbox
```

### Status Output

```
NAME                 IMAGE                                    RUNTIME    STATUS     RESTARTS   STARTED
image-resize         ghcr.io/iii-hq/image-resize:0.1.2       sandbox    running    0          2026-03-26T...
sentiment            ghcr.io/iii-hq/sentiment:0.3.0           docker     running    0          2026-03-26T...
```

### Engine Boot Output

```
Engine listening on address: 0.0.0.0:49134
Sandbox runtime: available (Docker Desktop 4.58+)
Starting managed workers...
Health check loop started (every 15s)
```

## Files Changed

### Delete
- `engine/src/cli/worker_manager/firecracker.rs`

### Create
- `engine/src/cli/worker_manager/sandbox.rs` — `SandboxAdapter` implementing `RuntimeAdapter`

### Modify
- `engine/src/cli/worker_manager/mod.rs` — replace `firecracker` module with `sandbox`, update `create_adapter` factory
- `engine/src/cli/managed.rs` — add `sandbox_available()` probe, update `start_managed_workers` to log availability
- `engine/src/main.rs` — update `--isolation strong` mapping from `firecracker` to `sandbox`

## Requirements

- Docker Desktop 4.58+ (for sandbox support)
- macOS or Windows (Linux: experimental container-based sandboxes via Docker Desktop 4.57+)

## Exit Criteria

1. `iii worker add image-resize --runtime sandbox` creates a sandbox, starts worker inside it
2. `iii worker status` shows `sandbox` runtime column
3. `iii worker stop/start/remove/logs` work for sandbox workers
4. Health check restarts crashed sandbox workers
5. Engine boot auto-detects sandbox availability
6. Engine boot skips sandbox workers with warning if sandbox unavailable
7. `docker sandbox run iii ~/project` works (engine-in-sandbox, no code changes needed)
