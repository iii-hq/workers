# Microsandbox Runtime — Design Spec

**Date:** 2026-03-27
**Status:** Approved
**Extends:** Docker Sandbox Runtime (2026-03-26)

## Summary

Add microsandbox as the primary strong-isolation runtime, with Docker Sandbox as fallback. Upgrade `iii.worker.yaml` to v2 format with structured runtime declarations. Auto-detect the best available runtime for both `iii worker dev` and `--isolation strong`.

Microsandbox provides sub-200ms microVM boot times, native language SDKs, and direct networking (no HTTP MITM proxy) — solving the WebSocket connectivity issue with Docker Sandbox.

## Design Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | Runtime priority | microsandbox > Docker Sandbox > Docker container | Microsandbox has best isolation + networking. Docker Sandbox is widely available. Docker container is last resort. |
| 2 | Manifest format | `iii.worker.yaml` v2, generate microsandbox config at runtime | Single source of truth. No `Sandboxfile` in projects. |
| 3 | Backward compatibility | v1 manifests still parsed | Existing projects continue to work during migration. |
| 4 | Microsandbox server | Required external dependency (`msb server start`) | Not bundled — user installs separately. Auto-detected on boot. |
| 5 | Engine URL | Set automatically by `worker dev` and managed worker start | Removed from manifest `env` section. Engine detects the right address per runtime. |

## `iii.worker.yaml` v2 Format

```yaml
iii: v2

name: image-resize-demo
version: 1.0.0

# Structured runtime — language + package_manager drive auto-detection
runtime:
  language: typescript    # typescript | python | rust | go
  package_manager: bun    # bun | npm | yarn | pnpm | pip | cargo
  entry: src/index.ts     # main file to run

# Resource limits for the sandbox VM
resources:
  memory: 512             # MB (plain number)
  cpus: 1                 # CPU cores

# Environment variables injected into the sandbox
# III_URL and III_ENGINE_URL are always set automatically
env:
  CUSTOM_VAR: "value"

# Optional: explicit commands when auto-detection isn't enough
scripts:
  setup: "curl -fsSL https://bun.sh/install | bash"
  install: "bun install"
  start: "bun src/index.ts"
```

### v2 Changes from v1

| Field | v1 | v2 |
|-------|----|----|
| Schema version | `iii: v1` | `iii: v2` |
| Commands section | `dependencies.setup/install/run` | `scripts.setup/install/start` |
| Memory format | `512Mi` (K8s-style) | `512` (plain MB) |
| Engine URL in env | Required manually | Auto-injected, removed from manifest |
| Scripts | Required | Optional — auto-detected from `runtime.language` + `runtime.package_manager` |

### Auto-Detection Rules

When `scripts` is omitted, `iii worker dev` infers commands from `runtime`:

| Language | Package Manager | Setup | Install | Start |
|----------|----------------|-------|---------|-------|
| typescript | bun | `curl -fsSL https://bun.sh/install \| bash` | `bun install` | `bun <entry>` |
| typescript | npm | install node via nodesource | `npm install` | `npx tsx <entry>` |
| python | pip | `python3 -m venv .venv` | `.venv/bin/pip install -e .` | `.venv/bin/python -m <entry>` |
| rust | cargo | install rustup + build-essential | `cargo build` | `cargo run` |

### Backward Compatibility

The parser checks `iii:` field:
- `v2` → parse v2 format
- `v1` or missing → parse v1 format (existing `dependencies.setup/install/run` fields)

## Runtime Selection

### `iii worker dev` Priority

```
1. Check `msb server status` → MicrosandboxAdapter
2. Check `docker sandbox ls` → SandboxAdapter
3. Check `docker info`       → DockerAdapter (container mode)
4. None available             → error
```

### `--isolation strong` Priority

```
1. Check `msb server status` → microsandbox
2. Check `docker sandbox ls` → sandbox
3. None available             → error (strong isolation required)
```

### `--runtime` Explicit Selection

```
--runtime docker        → DockerAdapter (always available)
--runtime sandbox       → SandboxAdapter (requires Docker Desktop)
--runtime microsandbox  → MicrosandboxAdapter (requires msb server)
```

## MicrosandboxAdapter

Implements `RuntimeAdapter` trait. Uses `msb` CLI and microsandbox server API.

### For Managed Workers (`iii worker add --runtime microsandbox`)

| Operation | Implementation |
|-----------|---------------|
| pull | OCI pull via Docker (images are runtime-agnostic) |
| extract_file | OCI extract via Docker |
| start | `msb exe --image <lang> --cpus N --memory M` then run worker binary inside |
| stop | Server API: stop sandbox |
| status | Server API: check sandbox running state |
| logs | Server API: get sandbox output |
| remove | Server API: destroy sandbox |

### For Dev (`iii worker dev` with microsandbox)

1. Detect language from `iii.worker.yaml`
2. Create sandbox: `msb exe --image <language> --cpus <N> --memory <M>`
3. Run setup script inside sandbox (if specified)
4. Run install script inside sandbox
5. Run start script with `III_URL=ws://<host>:<port>` — interactive, attached to terminal

### Networking Advantage

Microsandbox sandboxes have direct network access — no HTTP MITM proxy. The worker connects to `ws://localhost:<port>` (if engine runs on same host) or `ws://<lan-ip>:<port>` without proxy workarounds.

## Engine URL Strategy Per Runtime

| Runtime | Engine URL | Why |
|---------|-----------|-----|
| microsandbox | `ws://localhost:<port>` | Direct network access, no proxy |
| Docker Sandbox | `ws://<lan-ip>:<port>` | MITM proxy blocks WebSocket, bypass via LAN IP |
| Docker container | `ws://host.docker.internal:<port>` | Standard Docker networking |

## Files Changed

### Engine Repo — Create

| File | Responsibility |
|------|---------------|
| `engine/src/cli/worker_manager/microsandbox.rs` | `MicrosandboxAdapter` implementing `RuntimeAdapter` |

### Engine Repo — Modify

| File | Changes |
|------|---------|
| `engine/src/cli/worker_manager/mod.rs` | Add `microsandbox` module, update `create_adapter` factory |
| `engine/src/cli/managed.rs` | v2 manifest parser, auto-detect runtime selection for `worker dev`, language-based script inference |
| `engine/src/main.rs` | `--isolation strong` tries microsandbox first |

### Workers Repo — Modify

| File | Changes |
|------|---------|
| `image-resize/example/iii.worker.yaml` | Upgrade to v2 format |
| `image-resize/example-python/iii.worker.yaml` | Upgrade to v2 format |
| `image-resize/example-rust/iii.worker.yaml` | Upgrade to v2 format |

## Prerequisites

- Microsandbox: `curl -sSL https://get.microsandbox.dev | sh && msb server start --dev`
- Docker Desktop 4.58+ (for Docker Sandbox fallback)
- Docker (for standard container fallback)

## Exit Criteria

1. `iii worker dev ./example` auto-selects microsandbox when `msb` server is running
2. Falls back to Docker Sandbox when microsandbox unavailable
3. Falls back to Docker container when Docker Sandbox unavailable
4. `iii worker add <image> --runtime microsandbox` works for managed workers
5. `--isolation strong` tries microsandbox first, then Docker Sandbox
6. v2 `iii.worker.yaml` parsed correctly; v1 manifests still work
7. All three examples upgraded to v2 format
