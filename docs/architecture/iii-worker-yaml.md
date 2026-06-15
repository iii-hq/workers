# iii.worker.yaml contract

Every worker folder ships `iii.worker.yaml` at its root. CI discovery,
release routing, and registry publish all read this file.

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
| `bin` | Cargo binary name (defaults to `name`) | `_rust-binary.yml`, publish boot |
| `targets` | Optional list of Rust triples to build | `_rust-binary.yml` matrix subset; `supported_targets` in manifest |

When `targets` is omitted, all nine default triples are built: macOS
x86_64/aarch64, Windows x86_64/i686/aarch64, Linux x86_64 gnu/musl,
aarch64 gnu, and armv7 gnueabihf (the matrix lives in
[`_rust-binary.yml`](../../.github/workflows/_rust-binary.yml)).

## Opt-outs and runtime

| Field | Purpose | Consumer |
|---|---|---|
| `interface_smoke: false` | Skip interface boot smoke + registry publish | `ci.yml`, `release.yml` |
| `runtime` / `scripts.start` | Local boot definition; presence routes publish boot to `iii-add` for non-binary/bundle deploys | `manifest_version.py deploy-mode`, `iii worker add` |
| `scripts.install` | Build command for local install | `iii worker add` (local source) |

## Bundle-specific

| Field | Purpose |
|---|---|
| `dependencies` | Map of worker name → semver range for bundled sub-workers |

Example: [`harness/iii.worker.yaml`](../../harness/iii.worker.yaml).

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
- `parse_release_tag.py` rejects unknown `deploy` values at release time.

## Related

- Deploy routing: [`deploy-modes.md`](deploy-modes.md)
- Onboarding checklist: [`../sops/new-worker.md`](../sops/new-worker.md) §6
