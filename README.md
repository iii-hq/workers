# III Workers

Worker modules for the [III engine](https://github.com/iii-hq/iii).

Each worker is a standalone Rust binary that connects to the engine via WebSocket, registers functions and triggers, and uses the engine's built-in state. No custom SDKs — consume workers through iii-sdk (`iii.trigger()`).

## Workers

### image-resize

Image resize worker using stream channels.

**Supported formats:** JPEG, PNG, WebP (including cross-format conversion)

| Strategy | Behavior |
|---|---|
| `scale-to-fit` | Scales to fit within target dimensions, preserving aspect ratio (default) |
| `crop-to-fit` | Scales and center-crops to fill exact target dimensions |

```bash
cd image-resize && cargo build --release
./target/release/image-resize --url ws://127.0.0.1:49134
```

---

### sandbox-docker

Docker container sandbox. Creates isolated containers, executes commands, runs code, reads/writes files.

**Functions registered:**

| Function ID | Description |
|---|---|
| `sandbox::create` | Create a Docker container with security hardening |
| `sandbox::get` | Get sandbox details by ID |
| `sandbox::list` | List all sandboxes |
| `sandbox::kill` | Stop and remove a sandbox |
| `exec::run` | Execute a shell command inside a sandbox |
| `exec::code` | Write and run a code snippet (Python, JavaScript, Bash) |
| `fs::read` | Read a file from a sandbox |
| `fs::write` | Write a file to a sandbox |
| `fs::list` | List directory contents |

**Security:** `pids_limit=256`, `cap_drop=[NET_RAW, SYS_ADMIN, MKNOD]`, `no-new-privileges`, `network=none` by default.

```bash
cd sandbox-docker && cargo build --release
./target/release/sandbox-docker --url ws://127.0.0.1:49134
```

**Usage from iii-sdk:**

```typescript
const sbx = await iii.trigger({ functionId: 'sandbox::create', payload: { image: 'python:3.12-slim' } });
const result = await iii.trigger({ functionId: 'exec::run', payload: { id: sbx.id, command: 'echo hello' } });
const code = await iii.trigger({ functionId: 'exec::code', payload: { id: sbx.id, code: 'print(42)', language: 'python' } });
await iii.trigger({ functionId: 'sandbox::kill', payload: { id: sbx.id } });
```

**Configuration (`config.yaml`):**

```yaml
default_image: python:3.12-slim
default_timeout: 3600
default_memory: 512
max_sandboxes: 50
workspace_dir: /workspace
```

---

### sandbox-firecracker

KVM CoW fork sandbox for sub-millisecond VM spawning. Hardware-enforced isolation via Intel VT-x / AMD-V. **Linux-only.**

**How it works:** Loads a pre-built VM template (CPU state + memory dump) into a `memfd` at startup. Each `sandbox::create` call forks via `mmap(MAP_PRIVATE)` — Copy-on-Write memory mapping. Spawn time: ~0.8ms. Memory overhead: ~265KB per sandbox.

**Functions registered:**

| Function ID | Description |
|---|---|
| `sandbox::create` | Fork a new KVM VM from template (~0.8ms) |
| `sandbox::get` | Get sandbox details by ID |
| `sandbox::list` | List all sandboxes |
| `sandbox::kill` | Kill VM (munmap + close KVM fds) |
| `exec::run` | Execute command via serial I/O |
| `exec::code` | Run code snippet via serial I/O (Python, JavaScript, Bash) |

**Communication:** Serial I/O (16550 UART at COM1). No networking, no filesystem — code in, output out.

```bash
cd sandbox-firecracker && cargo build --release
./target/release/sandbox-firecracker --url ws://127.0.0.1:49134 --config config.yaml
```

**Usage from iii-sdk:**

```typescript
const vm = await iii.trigger({ functionId: 'sandbox::create', payload: { language: 'python' } });
const result = await iii.trigger({ functionId: 'exec::code', payload: { id: vm.id, code: 'print(42)' } });
await iii.trigger({ functionId: 'sandbox::kill', payload: { id: vm.id } });
```

**Configuration (`config.yaml`):**

```yaml
vmstate_path: ./template/vmstate
memfile_path: ./template/mem
mem_size_mb: 256
default_timeout: 30
max_sandboxes: 1000
```

**Prerequisites:** Linux with KVM enabled, pre-built Firecracker VM template (vmstate + memory dump).

---

## Building

Each worker is a standalone Rust crate:

```bash
cd <worker-name>
cargo build --release
cargo test
```

## CLI flags

All workers support:

| Flag | Default | Description |
|---|---|---|
| `--url` | `ws://127.0.0.1:49134` | III engine WebSocket URL |
| `--config` | `./config.yaml` | Path to config file |
| `--manifest` | — | Print module manifest JSON and exit |
