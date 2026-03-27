# Podman Runtime: Replace Docker with Podman as Default Container Runtime

**Date:** 2026-03-27
**Status:** Approved
**Scope:** Engine CLI — runtime adapters, managed workers, dev flow, CLI args

## Motivation

Replace Docker with Podman as the sole container runtime for iii workers:

- **No Docker Desktop license** — Podman is free/open-source, no commercial license restrictions
- **Daemonless architecture** — Podman runs rootless without a background daemon
- **OCI compliance** — Foundation for container-runtime agnostic future

## Decision Summary

| Area | Before | After |
|------|--------|-------|
| Default runtime | Docker | Podman |
| `DockerAdapter` | Core adapter | Removed, replaced by `PodmanAdapter` |
| `SandboxAdapter` | Docker Desktop microVMs | Removed entirely |
| `MicrosandboxAdapter` | Delegates to Docker for pull/extract | Delegates to Podman for pull/extract |
| `--isolation` flag | `standard` / `strong` with cascade | Removed |
| `--runtime` values | `docker`, `sandbox`, `microsandbox` | `podman` (default), `microsandbox` |
| `iii worker dev` | cascade: msb → sandbox → error | microsandbox only, error if unavailable |
| `iii start` managed | Always probes runtimes | Skips if no workers in `iii_workers` folder |
| macOS machine mgmt | N/A | Auto `podman machine init/start` |
| Host access URL | `host.docker.internal` | `host.containers.internal` (verify) |

## Architecture

### PodmanAdapter

Replaces `DockerAdapter`. Implements the same `RuntimeAdapter` trait. Since Podman is CLI-compatible with Docker for the commands we use, this is a direct swap of the CLI binary name.

**File:** `engine/src/cli/worker_manager/podman.rs` (replaces `docker.rs`)

**CLI command mapping** (all identical to Docker):

| Operation | Command |
|-----------|---------|
| Check image exists | `podman image inspect <image>` |
| Pull image | `podman pull <image>` |
| Get image size | `podman inspect --format "{{.Size}}" <image>` |
| Create temp container | `podman create --name <name> <image> true` |
| Copy file from container | `podman cp <container>:<path> -` |
| Remove container | `podman rm -f <container>` |
| Start container | `podman run -d --name <name> -e K=V [--memory M] [--cpus C] <image>` |
| Stop container | `podman stop --time <secs> <id>` |
| Check status | `podman inspect --format "{{.State.Running}}" <id>` |
| Get exit code | `podman inspect --format "{{.State.ExitCode}}" <id>` |
| Get container name | `podman inspect --format "{{.Name}}" <id>` |
| Get logs | `podman logs --tail 100 <id>` |

**`podman_available()` function:**
- Probes `podman --version`
- Returns `true` if Podman CLI is installed and responds

### macOS Machine Management

Podman on macOS requires a Linux VM (`podman machine`). The adapter auto-manages this:

1. Run `podman machine inspect` to check machine state
2. If **no machine exists** → `podman machine init` then `podman machine start`
3. If **machine exists but stopped** → `podman machine start`
4. If **running** → proceed

This check happens once at adapter creation time (cached result), not on every command.

### Removals

**Delete `SandboxAdapter` entirely:**
- Remove `engine/src/cli/worker_manager/sandbox.rs`
- Remove `pub mod sandbox;` from `worker_manager/mod.rs`
- Remove all `sandbox_available()` references from `managed.rs` and `main.rs`

**Delete `--isolation` flag:**
- Remove `isolation: Option<String>` CLI arg from `main.rs`
- Remove all isolation cascade logic

**Delete `run_dev_sandbox()` from `managed.rs`.**

### MicrosandboxAdapter Updates

The `MicrosandboxAdapter` currently delegates `pull()` and `extract_file()` to `DockerAdapter`. Update these to delegate to `PodmanAdapter` instead:

- `pull()` → `PodmanAdapter::pull()`
- `extract_file()` → `PodmanAdapter::extract_file()`

All other microsandbox operations (`msb` CLI) remain unchanged.

### create_adapter() Factory

```
match runtime {
    "microsandbox" => Arc::new(MicrosandboxAdapter::new()),
    _ => Arc::new(PodmanAdapter::new()),  // "podman" or default
}
```

Two adapters only. No Docker, no Sandbox.

## Dev Mode (`iii worker dev`)

Microsandbox only. No Podman in dev mode.

**`handle_worker_dev()` logic:**
1. Parse manifest from `iii.worker.yaml`
2. Check `msb_available()`
3. If available → `run_dev_microsandbox()`
4. If not → error: "microsandbox is required for dev mode. Install msb and start the server."

No `--runtime` override for dev mode. Dev is always microsandbox.

## Managed Workers (`iii start`)

**`start_managed_workers()` with early-exit guard:**

1. Scan the `iii_workers` folder for worker configs
2. If **no workers found** → return early, skip all runtime probing
3. If workers found → probe `podman_available()` / `msb_available()` as needed per worker
4. Start only the workers whose runtime is available

This avoids unnecessary `podman machine start` or `msb server status` calls when there's nothing to start.

**Engine URL for Podman workers:** `host.containers.internal` (Podman's native host-access hostname). If this doesn't work reliably in `podman machine` on macOS, fall back to LAN IP detection (same approach as microsandbox).

**Health loop:** Unchanged. `adapter.status()` / `adapter.stop()` / `adapter.start()` all route through the `RuntimeAdapter` trait — works with `PodmanAdapter` identically.

## CLI Changes (`main.rs`)

**`--runtime` flag:**
- Valid values: `podman` (default), `microsandbox`
- Default: `podman`

**Removed:**
- `--isolation` flag (arg definition + all resolution logic)

**`iii worker dev`:**
- Ignores `--runtime` — always microsandbox

**`iii add` / `iii start`:**
- Respects `--runtime` to select adapter
- Default `podman` flows through `create_adapter()`

## Files Changed

| File | Action |
|------|--------|
| `engine/src/cli/worker_manager/docker.rs` | Delete |
| `engine/src/cli/worker_manager/sandbox.rs` | Delete |
| `engine/src/cli/worker_manager/podman.rs` | Create (PodmanAdapter + machine mgmt) |
| `engine/src/cli/worker_manager/mod.rs` | Update factory, remove sandbox/docker mods |
| `engine/src/cli/worker_manager/microsandbox.rs` | Delegate to Podman instead of Docker |
| `engine/src/cli/managed.rs` | Remove sandbox dev, simplify cascade, guard managed start |
| `engine/src/main.rs` | Remove `--isolation`, update `--runtime` |

## Testing

- Unit tests for `PodmanAdapter` (mock CLI responses)
- Unit tests for machine management logic (state detection + init/start)
- Integration test: `podman_available()` probe
- Verify `host.containers.internal` resolves inside Podman containers on macOS
- Verify managed worker lifecycle: pull → start → health check → stop → remove
- Verify dev mode errors gracefully when msb unavailable
