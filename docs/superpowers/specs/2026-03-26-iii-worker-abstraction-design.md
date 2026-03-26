# iii Worker Abstraction Layer

**Date:** 2026-03-26
**Status:** Approved
**Approach:** Vertical slice (image-resize end-to-end, then generalize)

## Summary

Decouple the iii worker artifact and contract from the runtime substrate. Workers are packaged as OCI images with an embedded manifest, managed by a dedicated launcher worker that handles container lifecycle, and communicate with the engine over the existing WebSocket protocol.

One packaging story. One SDK story. One protocol story. Multiple execution substrates.

> An iii worker is an OCI image with an embedded manifest at `/iii/worker.yaml`, managed by a launcher worker that handles container lifecycle, communicating with the engine over the existing WebSocket protocol — packaged once, runnable anywhere.

## Design Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | Artifact format | OCI-only | One distribution story. Drop 9-platform binary matrix. containerd's runtime model is built around OCI. Firecracker ecosystem supports OCI via firecracker-containerd. |
| 2 | Transport | Keep WebSocket | Existing protocol is already network-native, bidirectional, supports binary frames. Three SDKs built on it. Transport change is not where the value is. |
| 3 | Container lifecycle | Dedicated launcher sidecar | Engine stays focused on orchestration. Launcher is a separate concern — testable, swappable, evolvable independently. |
| 4 | Manifest location | Embedded in OCI image at `/iii/worker.yaml` | Single source of truth. Always in sync with the code. No manifest-registry drift. |
| 5 | Launcher model | Hybrid — CLI bootstraps, then launcher is a regular iii worker | Dogfoods the protocol. Launcher capabilities are discoverable, invocable, observable. CLI handles trivial bootstrap. |
| 6 | Readiness | Manifest-driven — engine compares registered vs declared capabilities | Manifest and image are the same artifact, so they stay in sync by definition. No extra protocol message needed from the worker. |
| 7 | Self-hosted contract | Manifest optional — two tiers | Preserves backward compatibility. Managed workers require manifest. Self-hosted workers opt-in for readiness checking and capability discovery. |

## Two Deployment Modes

### Managed Mode

```bash
iii worker add <worker-name>
```

The engine (via the launcher worker) pulls the worker artifact, launches it in Docker/containerd or Firecracker, and auto-registers it with the iii instance.

### Self-Hosted Mode

User runs the worker however they want (Docker, ECS, Kubernetes, Fly, bare metal) and the worker registers itself with iii over the network.

```bash
docker run ghcr.io/iii-hq/image-resize:0.1.2 \
  -e III_ENGINE_URL=ws://engine.example.com:49134 \
  -e III_AUTH_TOKEN=...
```

The launcher is not involved. Worker connects, optionally sends manifest, registers functions.

---

## Worker Manifest Format

Lives at `/iii/worker.yaml` inside every OCI image.

```yaml
iii: v1                          # manifest schema version

name: image-resize
version: 0.1.2
description: "Image resize and format conversion"
author: iii-hq
license: MIT

entrypoint:
  command: ["/worker"]
  transport: websocket
  protocol: iii-worker-v1

capabilities:
  functions:
    - id: "image_resize::resize"
      description: "Resize an image via channel I/O"
      request_schema:
        type: object
        properties:
          input_channel: { type: object }
          output_channel: { type: object }
          metadata: { type: object }
        required: [input_channel, output_channel]
      response_schema:
        type: object
        properties:
          format: { type: string }
          width: { type: integer }
          height: { type: integer }

config:
  schema:
    type: object
    properties:
      width: { type: integer, default: 200 }
      height: { type: integer, default: 200 }
      strategy: { type: string, enum: [scale-to-fit, crop-to-fit], default: scale-to-fit }
      quality:
        type: object
        properties:
          jpeg: { type: integer, default: 85 }
          webp: { type: integer, default: 80 }

resources:
  memory: 256Mi
  cpu: "0.5"
```

### Manifest Fields

- **`iii`** — Schema version for forward compatibility. Currently `v1`.
- **`name`** — Worker identifier. Used in `iii worker add <name>` and engine registration.
- **`version`** — Semantic version. Must match the OCI image tag.
- **`entrypoint`** — How to start the worker. `transport` is `websocket` (current protocol). `protocol` is `iii-worker-v1` for version negotiation.
- **`capabilities.functions`** — Declares all functions the worker will register. The engine uses this for manifest-driven readiness: worker is ready when all declared functions are registered.
- **`config.schema`** — JSON Schema for runtime configuration. The launcher passes config to the container as environment or flags.
- **`resources`** — Hints for container resource allocation. Not hard limits — the launcher uses these for defaults.

### Manifest Generation

For Rust workers, the manifest is generated by the binary itself using the existing `--manifest` flag, extended to emit YAML:

```rust
pub fn generate_manifest() -> WorkerManifest {
    WorkerManifest {
        iii: "v1".into(),
        name: "image-resize".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: "Image resize and format conversion".into(),
        entrypoint: Entrypoint {
            command: vec!["/worker".into()],
            transport: "websocket".into(),
            protocol: "iii-worker-v1".into(),
        },
        capabilities: Capabilities {
            functions: vec![/* derived from handler registrations */],
        },
        config: ConfigSchema { /* from existing config.rs defaults */ },
        resources: Resources { memory: "256Mi".into(), cpu: "0.5".into() },
    }
}
```

For non-Rust workers, the manifest is hand-written or generated by SDK tooling.

---

## Protocol Extensions

Three new message types added to the existing WebSocket protocol. No breaking changes.

### New Messages

#### `WorkerManifest` (Worker -> Engine)

Sent immediately after WebSocket connection, before any `RegisterFunction` calls. Optional for self-hosted workers.

```json
{
  "type": "worker_manifest",
  "manifest": {
    "iii": "v1",
    "name": "image-resize",
    "version": "0.1.2",
    "capabilities": {
      "functions": ["image_resize::resize"]
    }
  }
}
```

Compressed form of the full manifest — enough for the engine to track expected capabilities and infer readiness.

#### `WorkerReady` (Engine -> Worker)

Sent by the engine when all declared functions from the manifest are registered.

```json
{
  "type": "worker_ready",
  "worker_id": "uuid",
  "functions_registered": ["image_resize::resize"]
}
```

Never sent for workers without a manifest — they are available immediately on connect (current behavior preserved).

#### `WorkerReadinessTimeout` (Engine -> Worker)

Sent if declared functions are not registered within a configurable timeout (default: 30s). Engine disconnects the worker after sending this.

```json
{
  "type": "worker_readiness_timeout",
  "worker_id": "uuid",
  "missing_functions": ["image_resize::resize"],
  "timeout_ms": 30000
}
```

### Modified Messages

#### `WorkerRegistered` (Engine -> Worker)

Add optional `manifest_accepted` field:

```json
{
  "type": "worker_registered",
  "worker_id": "uuid",
  "manifest_accepted": true
}
```

If the worker sent a manifest and it was valid, `manifest_accepted: true`. If malformed or unsupported schema version, `manifest_accepted: false` with an `error` field. Omitted entirely for workers that didn't send a manifest.

### Connection Sequences

#### Managed Worker (with manifest)

```
Worker                          Engine
  |                               |
  |--- WebSocket Connect -------->|
  |--- WorkerManifest ----------->|  (declares expected functions)
  |<-- WorkerRegistered ----------|  (manifest_accepted: true)
  |--- RegisterFunction --------->|  (image_resize::resize)
  |<-- WorkerReady ---------------|  (all declared functions registered)
  |                               |
  |<-- InvokeFunction ------------|  (now routable)
```

#### Self-Hosted Worker (no manifest)

```
Worker                          Engine
  |                               |
  |--- WebSocket Connect -------->|
  |<-- WorkerRegistered ----------|  (no manifest_accepted field)
  |--- RegisterFunction --------->|  (immediately available)
  |                               |
  |<-- InvokeFunction ------------|
```

---

## Launcher Worker (`iii-launcher`)

A dedicated iii worker that manages container lifecycle. The CLI bootstraps it as a local process, then it connects to the engine like any other worker.

### Architecture

```
+----------+         +-----------+         +---------------+
|  CLI     |  boot   | Launcher  |   ws    |    Engine      |
|  (iii)   |-------->| Process   |-------->|  (port 49134)  |
+----------+         +-----------+         +---------------+
                          |                       |
                     +----+------+                |
                     | Runtime   |                |
                     | Adapter   |                |
                     +----+------+                |
                          |                       |
                     +----+------+          +-----+-------+
                     | Docker/   |          |  Worker     |
                     | containerd|--------->| Container   |---ws--->
                     +-----------+          +-------------+
```

### Launcher Functions

| Function ID | Purpose |
|---|---|
| `iii_launcher::pull` | Pull OCI image, extract manifest from `/iii/worker.yaml`, return manifest contents |
| `iii_launcher::start` | Start worker container from OCI image with config and engine connection info |
| `iii_launcher::stop` | Stop a running worker container by name |
| `iii_launcher::status` | Return status of all managed worker containers |
| `iii_launcher::logs` | Stream logs from a managed worker container |

### Bootstrap Flow (`iii worker add image-resize`)

1. **CLI checks if launcher is running.** Queries the engine for a worker named `iii-launcher`. If not found, starts the launcher process locally.
2. **CLI invokes `iii_launcher::pull`.** Passes `ghcr.io/iii-hq/image-resize:0.1.2`. Launcher pulls the image, extracts `/iii/worker.yaml`, returns the manifest.
3. **CLI shows manifest to user.** Name, version, capabilities, resource hints. User confirms.
4. **CLI invokes `iii_launcher::start`.** Passes image ref, user-provided config (if any), engine URL + auth token for the worker to connect.
5. **Launcher starts container** via the active runtime adapter (Docker by default). Injects environment:
   - `III_ENGINE_URL=ws://host.docker.internal:49134`
   - `III_AUTH_TOKEN=...`
   - `III_WORKER_CONFIG=<base64-encoded YAML>`
6. **Worker container boots.** Connects to engine, sends manifest, registers functions.
7. **Engine confirms readiness.** Manifest-driven: all declared functions registered triggers `WorkerReady`.
8. **Launcher reports success.** CLI receives confirmation, updates local `iii.toml`.

### Runtime Adapter Interface

```rust
trait RuntimeAdapter {
    async fn pull(&self, image: &str) -> Result<ImageInfo>;
    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>>;
    async fn start(&self, spec: &ContainerSpec) -> Result<ContainerId>;
    async fn stop(&self, id: &ContainerId) -> Result<()>;
    async fn status(&self, id: &ContainerId) -> Result<ContainerStatus>;
    async fn logs(&self, id: &ContainerId) -> Result<LogStream>;
}
```

Phase 1 ships with `DockerAdapter`. Phase 2 adds `FirecrackerAdapter` via containerd. The adapter is selected by policy flag:

```bash
iii worker add image-resize                     # default: docker
iii worker add image-resize --runtime docker
iii worker add image-resize --runtime firecracker
iii worker add image-resize --isolation strong   # engine decides
```

### Launcher State

Tracked in `iii_workers/launcher-state.json`:

```json
{
  "managed_workers": {
    "image-resize": {
      "image": "ghcr.io/iii-hq/image-resize:0.1.2",
      "container_id": "abc123",
      "runtime": "docker",
      "started_at": "2026-03-26T13:00:00Z",
      "status": "running",
      "config": { "width": 200, "height": 200 }
    }
  }
}
```

---

## OCI Build Tooling

### Image Structure

```
/
+-- iii/
|   +-- worker.yaml          # manifest (required)
+-- worker                    # binary entrypoint
+-- ...                       # anything else the worker needs
```

The only hard requirement is `/iii/worker.yaml` at the well-known path.

### Dockerfile Pattern (Rust Workers)

```dockerfile
FROM rust:1.83-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release
RUN ./target/release/image-resize --manifest > /build/worker.yaml

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/image-resize /worker
COPY --from=builder /build/worker.yaml /iii/worker.yaml
ENTRYPOINT ["/worker"]
```

The manifest is generated by the binary itself — always derived from the actual code.

### CI/CD Workflow

Replace multi-platform binary releases with OCI image builds:

**Current:** `create-tag.yml` -> `release.yml` -> `_rust-binary.yml` (9 platform binaries -> GitHub Releases)

**New:** `create-tag.yml` -> `release.yml` -> `_oci-build.yml` (multi-arch OCI image -> container registry)

```yaml
# _oci-build.yml (reusable)
jobs:
  build-and-push:
    runs-on: ubuntu-latest
    steps:
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
      - uses: docker/build-push-action@v5
        with:
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ghcr.io/iii-hq/${{ inputs.worker_name }}:${{ inputs.version }}
```

Multi-arch (amd64 + arm64) covers practical deployment targets.

### Registry Format v2

Simplified from binary download URLs to image pointers:

```json
{
  "version": 2,
  "workers": {
    "image-resize": {
      "description": "Image resize and format conversion",
      "image": "ghcr.io/iii-hq/image-resize",
      "latest": "0.1.2"
    }
  }
}
```

Discovery index only. All metadata lives in the image manifest.

---

## SDK Changes

### Protocol Additions (All Three SDKs)

Optional `manifest` field on worker registration. When provided, the SDK:

1. Sends `WorkerManifest` message after connection
2. Waits for `WorkerReady` before resolving startup
3. Handles `WorkerReadinessTimeout` as startup error

When omitted, behavior is identical to today.

### Rust SDK

```rust
// Without manifest (current behavior, still works)
let iii = register_worker("ws://localhost:49134", WorkerConfig {
    worker_name: "image-resize".into(),
    ..Default::default()
}).await?;

// With manifest (opt-in)
let iii = register_worker("ws://localhost:49134", WorkerConfig {
    worker_name: "image-resize".into(),
    manifest: Some(generate_manifest()),
    ..Default::default()
}).await?;

// Auto-load from OCI image path
let iii = register_worker("ws://localhost:49134", WorkerConfig {
    worker_name: "image-resize".into(),
    manifest: WorkerManifest::from_file("/iii/worker.yaml").ok(),
    ..Default::default()
}).await?;
```

### Node.js SDK

```typescript
const iii = registerWorker('ws://localhost:49134', {
  workerName: 'image-resize',
  manifest: {
    iii: 'v1',
    name: 'image-resize',
    version: '0.1.2',
    capabilities: {
      functions: ['image_resize::resize']
    }
  }
});
```

### Python SDK

```python
manifest = WorkerManifest.from_file("/iii/worker.yaml")
iii = await register_worker("ws://localhost:49134",
    worker_name="sentiment-worker",
    manifest=manifest
)
```

### Worker Author Development Flow

```bash
# 1. Write handler code and manifest generation
# 2. Build and test locally (no Docker needed)
cargo run -- --url ws://localhost:49134

# 3. Build OCI image
docker build -t ghcr.io/myorg/my-worker:0.1.0 .

# 4. Test via managed mode
iii worker add ghcr.io/myorg/my-worker:0.1.0

# 5. Publish
docker push ghcr.io/myorg/my-worker:0.1.0
```

---

## CLI Changes

### New Commands

```bash
# Discovery
iii worker search [query]
iii worker inspect <image>

# Managed lifecycle
iii worker add <image> [--runtime docker|firecracker] [--isolation standard|strong]
iii worker remove <name>
iii worker start <name>
iii worker stop <name>

# Operational
iii worker status
iii worker logs <name> [--follow]
iii worker config <name>
iii worker config <name> --set key=value
```

### Shorthand Resolution

```bash
# Full image reference
iii worker add ghcr.io/iii-hq/image-resize:0.1.2

# From registry index (resolves name -> image ref)
iii worker add image-resize

# With version
iii worker add image-resize@0.1.2

# Latest
iii worker add image-resize@latest
```

### `iii worker add` Output

```
$ iii worker add ghcr.io/iii-hq/image-resize:0.1.2

Pulling image... done
Worker: image-resize v0.1.2
  Image resize and format conversion

Capabilities:
  Functions:
    - image_resize::resize

Resources:
  Memory: 256Mi
  CPU: 0.5

Proceed? [Y/n] y

Starting with runtime: docker
Container started: iii-image-resize-a1b2c3
Waiting for readiness... ready (1.2s)

Worker image-resize is running and registered with the engine.

Added to iii.toml:
  [workers]
  image-resize = "0.1.2"
```

### `iii worker status` Output

```
$ iii worker status

NAME            VERSION  RUNTIME  STATUS   FUNCTIONS  UPTIME
image-resize    0.1.2    docker   ready    1          2h 14m
sentiment       0.3.0    docker   ready    2          45m
code-sandbox    1.0.0    docker   failed   0/3        -

code-sandbox: readiness timeout - 3 functions declared, 0 registered
```

### `iii worker list` (Updated)

Shows both managed and self-hosted workers:

```
$ iii worker list

MANAGED WORKERS:
  image-resize    0.1.2    docker   ready

CONNECTED WORKERS (self-hosted):
  my-dev-worker   -        -        available   (no manifest)
  analytics       0.2.0    -        available   (manifest provided)
```

### Config File Changes

**`iii.toml`:**

```toml
[workers]
image-resize = { version = "0.1.2", image = "ghcr.io/iii-hq/image-resize:0.1.2", runtime = "docker" }
```

**`config.yaml`:** Worker config sections remain the same (delimited with `# === iii:<name> BEGIN/END ===`). The launcher reads these and passes them to containers.

---

## Phased Rollout

### Phase 1: Foundation (Vertical Slice)

Goal: `iii worker add image-resize` works end-to-end with Docker.

| Work Item | Repo | Description |
|---|---|---|
| Manifest format + types | `motia/engine` | Define `WorkerManifest` struct, YAML serde, validation |
| Protocol extensions | `motia/engine` | Add `WorkerManifest`, `WorkerReady`, `WorkerReadinessTimeout` messages to `protocol.rs` |
| Manifest-driven readiness | `motia/engine` | Engine tracks expected vs registered functions, infers readiness |
| Rust SDK manifest support | `motia/sdk` | Optional `manifest` field in `WorkerConfig`, auto-load from `/iii/worker.yaml` |
| image-resize OCI build | `workers` | Dockerfile, `--manifest` emits YAML, CI workflow (`_oci-build.yml`) |
| Launcher worker | `workers` (new) | `iii-launcher` worker with `DockerAdapter`, functions: pull/start/stop/status/logs |
| CLI managed commands | `motia/engine` | `iii worker add/remove/start/stop/status/logs` wired through launcher invocations |
| Registry v2 format | `workers` | Simplified `index.json` with image refs instead of binary download URLs |

**Exit criteria:** Run `iii worker add image-resize`, container starts, manifest-driven readiness confirms, function invocable, `iii worker status` shows it running.

### Phase 2: Multi-Runtime & Hardening

Goal: Firecracker support, isolation policies, production readiness.

| Work Item | Description |
|---|---|
| `FirecrackerAdapter` | containerd + Firecracker runtime shim integration in launcher |
| `--runtime` / `--isolation` flags | Policy-driven runtime selection |
| Resource enforcement | Launcher passes `resources` from manifest to container runtime |
| Graceful shutdown | Engine sends shutdown signal, worker drains in-flight invocations, container stops |
| Health check loop | Launcher periodically checks container health, restarts on failure |
| Node.js + Python SDK manifest support | Parity with Rust SDK |

**Exit criteria:** Same worker image runs in both Docker and Firecracker. `--isolation strong` routes to Firecracker. Workers restart on crash.

### Phase 3: Trust & Marketplace

Goal: Safe execution of third-party workers.

| Work Item | Description |
|---|---|
| Image signing | Workers can be signed (cosign/notation), launcher verifies signatures |
| Capability permissions | Manifest declares what engine services the worker can access |
| Trust tiers | `verified` (signed by known publisher), `signed` (any valid signature), `sandbox-required` (must run in Firecracker) |
| Registry publishing flow | `iii worker publish` pushes image + updates registry |
| `iii worker search` improvements | Search by capability, language, trust tier |

**Exit criteria:** Third-party workers can be published, discovered, and run with appropriate isolation based on trust level.
