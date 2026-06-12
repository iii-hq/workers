# Deploy modes

How `iii.worker.yaml` `deploy` routes CI interface smoke and release builds.

## Overview

| `deploy` | Artifact | Build workflow | Typical language |
|---|---|---|---|
| `binary` | Per-target CLI archives on GitHub Release | `_rust-binary.yml` | Rust |
| `image` | `ghcr.io/<owner>/<worker>:<version>` | `_container.yml` | Node, Python |
| `bundle` | `<worker>.tar.gz` on GitHub Release | `_bundle.yml` | Node (esbuild) |

Release dispatcher: [`release.yml`](../../.github/workflows/release.yml) reads
`deploy` from `iii.worker.yaml` via `parse_release_tag.py`.

## Binary

- **Build:** up to 9 cross-compiled targets (or `targets:` subset).
- **Assets:** `<bin>-<triple>.tar.gz` / `.zip` + `.sha256` checksums.
- **Publish boot:** downloads `*-x86_64-unknown-linux-gnu.tar.gz` from the
  Release, runs the binary for interface collection.
- **Registry payload:** per-target download URLs resolved by
  `resolve_binary_artifacts.py`.

## Image

- **Build:** multi-arch Docker image pushed to GHCR.
- **Publish boot:** from local source via `iii worker add ./<worker>` (the
  image itself is not booted; `runtime`/`scripts.start` drive the local run).
- **Registry payload:** the built image reference (`image_tag` output).

Templates: [`todo-worker/`](../../todo-worker/), [`todo-worker-python/`](../../todo-worker-python/).

## Bundle

- **Build:** esbuild single-file `index.mjs` + `iii.worker.yaml` packed into
  `<worker>.tar.gz`.
- **Asset URL:** `https://github.com/<repo>/releases/download/<tag>/<worker>.tar.gz`
- **Publish boot:** extracts bundle, runs `node ./index.mjs`.
- **Dependencies:** bundled worker may declare in-repo deps in `iii.worker.yaml`
  `dependencies`; when the bundle worker's source changes, those deps join the
  CI matrix (see [`testing-and-ci.md`](testing-and-ci.md)).

Example: [`harness/`](../../harness/).

## Publish boot modes

[`manifest_version.py deploy-mode`](../../.github/scripts/manifest_version.py)
selects how `_publish-registry.yml` starts the worker:

| Mode | When | Boot |
|---|---|---|
| `release-binary` | `deploy: binary` | Download + run Linux gnu binary from Release |
| `release-bundle` | `deploy: bundle` | Extract tarball, `node ./index.mjs` |
| `iii-add` | Other deploys with `runtime` or `scripts.start` | `iii worker add ./<worker>` |
| `cargo-run` | Other deploys, Rust, no `runtime`/`scripts.start` | `cargo run` (+ `config.collect.yaml` if present) |

## config.collect.yaml

Workers whose default `config.yaml` spawns sidecars or requires local paths
unavailable on CI ship `config.collect.yaml` — a lighter config used only for
interface collection at publish time and in the `interface-smoke` CI job.

Precedents: `storage`, `shell`, `coder`, `database`.

## Related

- Release SOP: [`../sops/release.md`](../sops/release.md)
- Testing: [`testing-and-ci.md`](testing-and-ci.md)
