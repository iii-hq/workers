# Sandbox Worker Family

The set of iii workers that expose ephemeral, isolated execution environments to callers via the `sandbox::*` trigger family. The reference implementation is `iii-sandbox` (libkrun microVM, shipped in `iii-worker`). Provider-backed members (`sandbox-e2b`, `sandbox-daytona`, `sandbox-morph`, `sandbox-vercel`, `sandbox-modal`, `sandbox-cloudflare`) wrap external services and conform to the same ABI.

## Language

**Sandbox**:
A caller-facing handle to an ephemeral, isolated execution environment with a public id. Backing implementation may be a libkrun microVM owned by iii itself or a provider-managed sandbox / instance / container reached via REST or SDK. The worker layer normalizes provider terminology — Morph's "instance", Cloudflare's "container", E2B's "sandbox" all surface as `sandbox_id`.
_Avoid_: instance, container, vm, workspace.

**Sandbox Worker**:
An iii worker that registers `sandbox::<provider>::*` triggers and translates between the canonical Sandbox ABI and a specific backing implementation.
_Avoid_: provider, adapter, integration.

**Image**:
Provider-specific opaque string identifying the rootfs / template / snapshot a sandbox boots from. Each provider interprets it differently — E2B template id, Daytona snapshot ref, Morph snapshot UUID, libkrun OCI ref. The caller is responsible for knowing what their target provider expects.
_Avoid_: template, runtime (Vercel-specific), snapshot (see below).

**Snapshot**:
A capture of a Sandbox's state that can be restored later. Returns a `snapshot_id` that is opaque to the caller and meaningful only to the worker that produced it. Semantics differ per provider (E2B pause = memory + fs; Daytona = fs as OCI image; Morph = running VM with live process state; Modal = filesystem only). Restore is not yet uniform across the family.
_Avoid_: pause, checkpoint, save.

**Capabilities**:
The list of optional functions a Sandbox Worker registers. Returned from `create` as `capabilities[]`. Advisory — callers should inspect it before invoking optional functions. Calling an unregistered function returns the engine's standard "function not found" error.
_Avoid_: features, supported_methods.

**S-code**:
Stable error code from a Sandbox Worker. The shared space is inherited from `iii-sandbox`:
- `S100` image not in allowlist
- `S200` resource oversize
- `S300` host can't boot (libkrun: no KVM) — REST workers do not emit
- `S400` concurrency cap reached

REST workers add:
- `S404` capability not supported
- `S500` rate-limited (provider 429)
- `S501` quota exhausted (provider 402)
- `S502` provider unavailable (5xx or unparseable)
- `S503` auth invalid (401 / 403)

## ABI

The reference implementation is `iii-sandbox`. The lifecycle floor is required for every member; extensions are optional and capability-gated.

### Required (lifecycle floor)

| Function | Input | Output |
|---|---|---|
| `sandbox::<provider>::create` | `{image, idle_timeout_secs?, ...}` | `{sandbox_id, image, capabilities[], started_at}` |
| `sandbox::<provider>::exec` | `{sandbox_id, cmd, args?, env?, cwd?, timeout_ms?}` | `{stdout, stderr, exit_code, timed_out}` |
| `sandbox::<provider>::stop` | `{sandbox_id}` | `{}` |
| `sandbox::<provider>::list` | `{}` | `{sandboxes[], in_flight, cap, remaining, reconciled}` |

### Optional (capability-gated)

| Capability | Function | Notes |
|---|---|---|
| `snapshot` | `sandbox::<provider>::snapshot` | Returns provider-scoped opaque `snapshot_id` |
| `branch` | `sandbox::<provider>::branch` | Spawn N siblings; only Morph supports today |
| `expose_port` | `sandbox::<provider>::expose_port` | Returns public URL |
| `fs` | `sandbox::<provider>::fs::read`, `sandbox::<provider>::fs::write` | Channel-based bytes |

### Namespace shape

Lifecycle is flat (`sandbox::<provider>::create`); sub-resources nest (`sandbox::<provider>::fs::read`, `sandbox::<provider>::process::kill`). Provider segment is required to avoid collision with `iii-sandbox` which owns the bare `sandbox::*` namespace.

### Idempotency

`stop` is idempotent w.r.t. observed post-state. Any path where the worker can confirm "not running" returns success — including upstream `404` (sandbox already gone) and `409` (deletion in progress). `iii-sandbox` currently errors on missing-from-registry; this is a known divergence tracked as a follow-up.

### Implementation freedom

A worker MAY use any iii primitive: `registerFunction`, `registerTrigger` (http / cron / pubsub / queue), `iii.trigger`, `state::*`. The trigger ABI is the contract; implementation is free. External transports (HTTP, gRPC, SDK calls, deployed CF Worker bridges) are implementation detail.

### Lifetime knob

`idle_timeout_secs` (seconds) is the canonical lifetime field for v0. Semantics are provider-shaped — hard cap on E2B / Morph / Vercel / Modal; idle-reset on Cloudflare / Daytona. A future iteration may add `max_lifetime_secs` to disambiguate.

## Relationships

- Every **Sandbox Worker** registers a subset of the **ABI** under `sandbox::<provider>::*`
- Every **Sandbox** declares its supported optional **Capabilities** in the `create` response
- A **Snapshot** is produced by exactly one **Sandbox Worker** and is meaningful only to that worker
- `iii-sandbox` is the reference; provider workers SHOULD track its lifecycle floor and MUST NOT shadow the bare `sandbox::*` namespace

## ABI evolution policy

The lifecycle floor (`create`, `exec`, `stop`, `list`) is a hard requirement for every member of the family. Extensions are optional and may be added or removed independently. `iii-sandbox` is the reference; new lifecycle functions there create a "should follow" expectation on provider workers, but capability gating gives reasonable excuse for incomplete coverage.

Each worker is also expected to grow toward full provider-native parity over time. The v0 floor (lifecycle + minimal extensions) is a starting point, not the final surface.

## Example dialogue

> **Caller dev:** "I want to spawn a sandbox, run npm install, then stop it."
> **iii dev:** "Pick a provider. `iii worker add sandbox-e2b`. Set `E2B_API_KEY`. Then `iii.trigger('sandbox::e2b::create', {image: 'base'})` → `iii.trigger('sandbox::e2b::exec', {sandbox_id, cmd: 'npm', args: ['install']})` → `iii.trigger('sandbox::e2b::stop', {sandbox_id})`."
> **Caller dev:** "What if I want to use Modal instead?"
> **iii dev:** "`iii worker add sandbox-modal`, set `MODAL_TOKEN_ID` + `MODAL_TOKEN_SECRET`, swap the function-id prefix. The `image` field changes meaning per provider — Modal expects a Modal image ref, E2B expects a template id — so check the worker's README. Lifecycle functions and response shapes match."
> **Caller dev:** "Can I `branch` a sandbox?"
> **iii dev:** "Only on Morph today. Inspect `capabilities[]` from the `create` response — if `branch` isn't in it, the function isn't registered."

## Flagged ambiguities

- **"Image"** is overloaded across providers. Each worker's README documents what its `image` field accepts. Cross-provider portability of the same string is not guaranteed.
- **"Snapshot"** is overloaded. `snapshot_id` is opaque and worker-scoped. Restore semantics differ per provider; v0 leaves restore to provider-specific paths.
- **Vercel asymmetry** — Vercel uses `source` (git ref) + `runtime` (`node24` / `python3.13`) instead of `image`. Callers pass `source_url` / `source_revision` in the create payload; the canonical `image` field is informational on Vercel.
- **Cloudflare topology** — `sandbox-cloudflare` ships two artifacts: a Node iii worker AND a CF Worker bridge deployed via wrangler. The bridge is implementation detail; callers don't see it.
- **Branch as first-class** — only Morph today. If future providers add a comparable primitive, `branch` semantics will need a canonical specification (live-state-preserving vs filesystem-only fan-out).
