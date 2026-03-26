# Phase 2A: Runtime Hardening

**Date:** 2026-03-26
**Status:** Approved
**Prerequisite:** Phase 1 (Worker Abstraction Layer) complete on `feat/worker-abstraction-layer`

## Summary

Harden the Docker-based managed worker runtime with resource enforcement from manifests, a health check loop with exponential backoff restarts, and graceful shutdown via Docker SIGTERM + SDK invocation draining.

## Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | Health definition | Container running + worker registered with engine | Catches crashed containers and stuck workers without requiring a new ping protocol |
| 2 | Graceful shutdown | Docker SIGTERM + SDK drains in-flight invocations | No protocol changes. Unix-native. SDK already tracks in-flight invocations. |
| 3 | Restart policy | Fixed interval (15s), exponential backoff (5s→60s), max 5 retries, structured logging | Prevents restart storms, gives visibility via `iii worker status` |

---

## 1. Resource Enforcement

### Problem

The manifest declares `resources.memory` and `resources.cpu`. The `DockerAdapter` already supports `--memory` and `--cpus` flags. But the pull → start pipeline doesn't propagate manifest resources to the container spec.

### Changes

**CLI (`managed.rs`):** After pulling the manifest, extract `resources.memory` and `resources.cpu` and pass them in the start request payload as `memory_limit` and `cpu_limit`.

```
Pull manifest → extract resources → pass to start request → DockerAdapter passes to docker run
```

The manifest values serve as defaults. If the user passes explicit `--memory` or `--cpus` CLI flags, those override the manifest values.

No new types, no new protocol messages. Pure wiring.

---

## 2. Health Check Loop

### Architecture

The launcher spawns a background `tokio::spawn` task in `main.rs` after all functions are registered. It holds:
- `Arc<dyn RuntimeAdapter>` — to check container status
- `Arc<Mutex<LauncherState>>` — to read/write managed worker state
- SDK handle — to call `list_workers()` on the engine

### Health Check Logic (every 15 seconds)

For each managed worker in state where `status != "failed"`:

1. **Container check:** `adapter.status(container_id)` → is `running` true?
2. **Engine check:** `iii.list_workers()` → is a worker with matching name connected?
3. Both pass → healthy. Reset `restart_count` to 0, clear `backoff_until`.
4. Either fails → unhealthy. Enter restart flow.

### Restart with Exponential Backoff

On unhealthy:

1. Increment `restart_count`.
2. If `restart_count > 5` → set `status = "failed"`, set `last_failure` with reason, log structured error, skip further checks.
3. Compute backoff: `min(5 * 2^(restart_count - 1), 60)` seconds.
4. Set `backoff_until = now + backoff`.
5. If current time < `backoff_until` → skip (in cooldown).
6. Stop old container (`adapter.stop` + `adapter.remove`).
7. Start new container with same spec (same image, env, resource limits).
8. Update `container_id` in state, save state.
9. Log: `tracing::warn!(worker = name, restart_count = count, reason = reason, "restarting unhealthy worker")`.

### State Changes

`ManagedWorker` gets three new fields:

```rust
pub restart_count: u32,           // default: 0
pub last_failure: Option<String>, // default: None
pub backoff_until: Option<DateTime<Utc>>, // default: None
```

Serialized to `launcher-state.json`. Survives launcher restarts.

A successful health check (both container and engine checks pass) resets `restart_count` to 0 and clears `last_failure` and `backoff_until`.

### Status Output

`iii worker status` shows restart info:

```
NAME            RUNTIME  STATUS    RESTARTS  UPTIME
image-resize    docker   running   0         2h 14m
sentiment       docker   running   2         5m
code-sandbox    docker   failed    5         -

code-sandbox: max restarts exceeded — last failure: container exited with code 1
```

---

## 3. Graceful Shutdown

### Docker Stop Timeout

`ContainerSpec` gets a new field:

```rust
pub stop_timeout_secs: Option<u32>,  // default: 30
```

`DockerAdapter::stop()` passes it to Docker:

```
docker stop --time 30 <container>
```

Docker sends SIGTERM, waits 30 seconds, then SIGKILL.

### SDK Shutdown Drain

The Rust SDK's shutdown path changes from "exit immediately" to:

1. On SIGTERM: set a `shutting_down` flag.
2. Stop accepting new invocations — `InvokeFunction` messages received after the flag is set are rejected with an error response.
3. Wait for in-flight invocations to drain. The SDK already tracks these in `invocations: Arc<RwLock<HashSet<Uuid>>>`. Poll until the set is empty.
4. Drain timeout: 25 seconds (5s buffer before Docker's 30s SIGKILL deadline).
5. Disconnect WebSocket and exit.

### Stop Flow

```
CLI: iii worker remove image-resize
  → Launcher: adapter.stop("image-resize")
    → Docker: docker stop --time 30 image-resize
      → Container receives SIGTERM
        → SDK stops accepting new invocations
        → SDK waits for in-flight to complete (max 25s)
        → SDK disconnects WebSocket
        → Process exits cleanly
      → Docker confirms stop
  → Launcher: adapter.remove("image-resize")
  → Launcher: update state, save
```

### No Engine Changes

The engine doesn't participate in graceful shutdown. From its perspective, the worker disconnects normally (existing cleanup: unregister functions, clean up invocations). The drain happens between Docker and the worker process.

### Health Check Restart Flow

When the health check restarts a worker, it uses the same graceful flow:

```
Health check detects unhealthy
  → adapter.stop(container_id)    # SIGTERM + 30s grace
  → adapter.remove(container_id)  # clean up
  → adapter.start(same_spec)      # restart
  → update state
```

---

## Scope

### In Scope
- Manifest resource propagation from pull to start
- Health check background loop (15s interval)
- Restart with exponential backoff (5 retries, 5s→60s)
- `ManagedWorker` state enrichment (restart_count, last_failure, backoff_until)
- Docker stop timeout (30s configurable)
- SDK shutdown drain (reject new invocations, wait for in-flight, 25s timeout)
- `iii worker status` shows restart info

### Out of Scope
- Firecracker adapter (Phase 2B)
- `--runtime` / `--isolation` flags (Phase 2B)
- Node.js / Python SDK manifest support (Phase 2C)
- Worker-initiated shutdown (worker decides to stop itself)
- Custom health check endpoints (beyond container + engine registration)
