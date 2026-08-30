# iii.worker.yaml contract

Every worker folder ships `iii.worker.yaml` at its root. It is the public iii
contract for local development, scaffolding, installation, and package
consumers. The Workers release compiler reads it once to produce an immutable
descriptor; Release Control and post-prepare workflows never read it.

## Required fields

| Field | Type | Purpose |
|---|---|---|
| `iii` | `v1` | Schema version |
| `name` | string | Folder name; git tag prefix; registry id |
| `language` | enum | `rust` \| `javascript` \| `node` \| `python` — routes CI language job |
| `deploy` | enum | `binary` \| `image` \| `bundle` — routes release build + publish |
| `manifest` | path | Version source: `Cargo.toml`, `package.json`, `pyproject.toml` |
| `description` | string | Registry + `--manifest` output |

## Binary-specific

| Field | Purpose | Consumer |
|---|---|---|
| `bin` | Cargo binary name (defaults to `name`) | private release parity, local boot |
| `targets` | Optional public Rust target list | parity with `.deploy/workers.yaml` when present |

When `targets` is omitted, all six default Unix triples are built: macOS
x86_64/aarch64, Linux x86_64 gnu/musl, aarch64 gnu, and armv7 gnueabihf.
The authoritative release matrix lives in `.deploy/workers.yaml`; when this
public field is present the compiler requires the two lists to match.

## Opt-outs and runtime

| Field | Purpose | Consumer |
|---|---|---|
| `interface_smoke: false` | Opt out of the PR-only interface boot check | `validate_worker.py`, `ci.yml` |
| `runtime` / `scripts.start` | Public local and installed runtime definition | `iii worker`, Compose, release compiler |
| `scripts.install` | Build command for local install | `compose::add` (local source) |
| `tags` | Discovery aliases included in the compiled Registry projection | release compiler |

`tags` must be a list of strings. The publish pipeline trims values, converts them to lowercase,
removes duplicates, and omits the field when no non-empty tags remain.
`interface_smoke: false` skips only the PR check; deployments never boot a
prepared worker. Private `publish` controls whether a worker appears in the
deployment index.

```yaml
tags:
  - http
  - rest
  - api
```

## Bundle-specific

| Field | Purpose |
|---|---|
| `dependencies` | Map of worker name → semver range for bundled sub-workers |

Example: [`harness/iii.worker.yaml`](../../harness/iii.worker.yaml).

Engine-owned dependencies such as `configuration`, `iii-stream`, and
`iii-observability` use npm-style major wildcards such as `0.x`. Published stable
worker dependencies use a caret range whose lower bound is the newest stable
release validated with the current SDK. For example, `state: "^0.22.2"` accepts
compatible patch releases without falling back to an older SDK build. A worker
installed explicitly from the `next` channel may temporarily use a newer
target lower bound, but stable dependents must wait until that target reaches
`latest`. Experimental workers may retain broad ranges until their dependency
stack is promoted. These forms are understood directly by iii and the Registry.

## Example (minimal Rust binary)

```yaml
iii: v1
name: session-manager
language: rust
deploy: binary
manifest: Cargo.toml
bin: session-manager
description: Durable, reactive, branching store of typed conversation entries.
```

## Example (POSIX-only targets)

```yaml
targets:
  - x86_64-apple-darwin
  - aarch64-apple-darwin
  - x86_64-unknown-linux-gnu
  - x86_64-unknown-linux-musl
  - aarch64-unknown-linux-gnu
  - armv7-unknown-linux-gnueabihf
```

See [`shell/iii.worker.yaml`](../../shell/iii.worker.yaml).

## Validation

- `pr-checks` parses the file via [`validate_worker.py`](../../.github/scripts/validate_worker.py).
- `deployment_compiler.py` rejects public/private deploy-shape mismatches.

## Related

- Deploy routing: [`deploy-modes.md`](deploy-modes.md)
- Onboarding checklist: [`../sops/new-worker.md`](../sops/new-worker.md) §6
