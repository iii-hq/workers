# sandbox-builtin

The builtin sandbox adapter: it runs sandboxes and worker VMs as libkrun
microVMs on the engine's host. The engine's `sandbox::*` router forwards each
sandbox call to it as `sandbox-builtin::<operation>`, and Compose boots
`package://` and `path://` VM workers through its `sandbox-builtin::worker::*`
functions. You rarely call it directly: callers use `sandbox::*`, and Compose
installs this adapter when a project needs it.

## Install

Compose adds it on its own when a project needs sandboxes or VM workers. To
add it by hand on iii 0.26 or later, use the `iii-sandbox` alias, which pins
the adapter to the sandbox contract major the engine speaks:

```bash
iii trigger compose::add worker=iii-sandbox
```

Older Compose has no such alias, so pin the major yourself:

```bash
iii trigger compose::add worker=sandbox-builtin@^1
```

Calling `sandbox::*` through the router needs iii 0.26 or later; the
`sandbox-builtin::*` ids work on iii 0.24 and later.

## Quickstart

Run code in a fresh VM through the router, which forwards to this adapter:

```bash
iii trigger sandbox::run --timeout-ms 300000 \
  --json '{"image":"python","code":"print(6*7)","lang":"python"}'
```

The reply carries `stdout` (`42`), `stderr` and `exit_code`; the VM stops
when the call returns. The first call pulls the image, so it is slower. `sandbox-builtin::run` takes the same payload directly.

## What it serves

| Functions | Purpose |
|---|---|
| `create`, `exec`, `stop`, `list`, `catalog::list`, `run` | Sandbox lifecycle and one-call runs |
| `fs::ls`, `fs::stat`, `fs::mkdir`, `fs::rm`, `fs::chmod`, `fs::mv`, `fs::grep`, `fs::sed`, `fs::write`, `fs::read` | Files inside a live sandbox |
| `adapter::info` | Adapter name, version and the sandbox contract version it implements |
| `worker::prepare`, `worker::start`, `worker::status`, `worker::logs`, `worker::stop`, `worker::destroy` | Compose's worker VMs (see below) |

Every id is prefixed with `sandbox-builtin::`.

## Configuration

The adapter reads its configuration entry once at startup with
`configuration::get` (`III_CONFIG_NAME`, else `sandbox-builtin`). A change
applies when the adapter restarts. A config that does not parse, or a failed
read, stops the adapter with exit code 2 instead of falling back to defaults.

```yaml
auto_install: true              # pull a missing image on first create
image_allowlist: [python, node] # bootable images; empty denies every create
default_idle_timeout_secs: 300  # idle sandboxes are reaped after this
max_concurrent_sandboxes: 32
max_concurrent_exec_per_sandbox: 4  # 0 means 1, not unlimited
max_exec_timeout_ms: 300000     # ceiling on any exec's timeout_ms
default_cpus: 1
default_memory_mb: 512
per_image_caps:                 # optional per-image ceilings
  python: { max_cpus: 4, max_memory_mb: 2048 }
custom_images:                  # extra images: name -> OCI ref
  my-app: ghcr.io/acme/my-app:1.2.3   # must also be in image_allowlist
```

A `custom_images` key may not reuse a preset name (`python`, `node`), and the
old wrapped shape (`sandbox: {...}`) is rejected.

VM tuning stays in four environment variables the adapter passes to each
worker VM (sandboxes ignore them):

| Variable | Effect |
|---|---|
| `III_VM_HYPERTHREADING=1` | Enable SMT (hyperthreading) in the guest |
| `III_VM_NESTED_VIRT=1` | Enable nested virtualization |
| `III_VM_VIRTIOFS_SHM_SIZE_MIB=<n>` | virtiofs shared-memory window; `0` skips it |
| `III_VM_NOFILE_LIMIT=<n>` | Guest open-file limit; `0` leaves it to the guest init |

## Host prerequisites

- **macOS, Apple Silicon only.** The released binary is ad-hoc signed with
  the `com.apple.security.hypervisor` and
  `com.apple.security.cs.disable-library-validation` entitlements. An
  unsigned dev build signs itself in place on first run.
- **Linux, glibc.** The Compose user needs read/write access to `/dev/kvm`,
  and `libcap-ng0` must be installed.
- Any other host gets a clear `UnsupportedPlatform` error from Compose when a
  project needs the adapter.

## Worker VMs

Compose runs VM workers through `sandbox-builtin::worker::*`. Because those
requests name host directories (`host_path`) and carry the guest's
environment, two guards apply:

- **Token.** Compose generates a per-project token and passes it to the
  adapter as `III_SANDBOX_WORKER_TOKEN`. Every `worker::*` request carries it
  in `auth`; a mismatch gets S505.
- **Host roots.** `host_path` is accepted only under the project's Compose
  directory and the Compose package cache. Anything else gets S501 naming the
  two roots.

Compose also sets `III_DISABLE_TRACE_PAYLOADS=1` on the adapter, so the token
and guest env never land in the engine's trace store.

## Known limits

The guards assume bus clients are trusted. A client that registers the
adapter's function ids or its worker name first, while the adapter is down,
receives the `auth` token and the guest `env` that Compose sends. The router
has the same limit for `sandbox::*` forwards.

## Errors

Errors carry an S-code with a `docs_url`. The full list is at
[iii.dev/docs/creating-workers/sandboxes](https://iii.dev/docs/creating-workers/sandboxes).
