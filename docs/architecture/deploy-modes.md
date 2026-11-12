# Release artifact modes

The Workers-owned compiler,
[`deployment_compiler.py`](../../.github/scripts/deployment_compiler.py),
turns a worker's private build entry in
[`.deploy/workers.yaml`](../../.deploy/workers.yaml) plus its public
`iii.worker.yaml` into an immutable deployment descriptor with independent
build units.
[`deploy-descriptor-index.yml`](../../.github/workflows/deploy-descriptor-index.yml)
compiles every worker at one source SHA. This repository builds and never
publishes: Release Control publishes the built bytes. The operator procedure is
[`../sops/release.md`](../sops/release.md).

| `artifact.kind` | Built artifact | Where the bytes go |
|---|---|---|
| `rust-binary`, `go-binary` | one deterministic `tar.gz` per target (`zip` on Windows) | assets of the `build-<source_sha>` GitHub prerelease |
| `javascript-bundle`, `python-bundle` | one deterministic `tar.gz` from explicit files | assets of the `build-<source_sha>` GitHub prerelease |
| `oci-image` | one OCI layout per platform, built twice to prove it is reproducible | an OCI index pushed to `ghcr.io/<owner>/<worker>:build-<source_sha>` |

Binary workers declare one `targets` list; the policy behind it lives in
[`deployment_targets.py`](../../.github/scripts/deployment_targets.py). A
worker without Windows targets must justify their absence with
`windows_exception`, and declaring both fails compilation.

[`build.yml`](../../.github/workflows/build.yml) builds one worker at one exact
source SHA. It builds embedded `frontends` once, then runs one
[`_deploy-build.yml`](../../.github/workflows/_deploy-build.yml) job per build
unit. Rust units use a remote sccache keyed by toolchain and target when that
backend is available, but artifact bytes remain target-specific. The build
uploads each asset once (identical bytes are skipped, different bytes fail the
job) and writes an attested `build-manifest` listing every asset's URL and
SHA-256.

Release Control reads that manifest, publishes the exact Registry version,
moves the requested channel by compare-and-swap, creates the versioned GitHub
release pointing at the same assets and tags the image; nothing is rebuilt. It
spreads the descriptor's `registry_projection` (worker name, type, description,
license, tags, dependencies, config, experimental flag, README, skills, and
`iii_engine_version` when set) into the Registry's `POST /publish`. After each
GitHub release it creates,
[`sync-manifest-versions.yml`](../../.github/workflows/sync-manifest-versions.yml)
writes every worker's highest published version back into its package manifests
and lockfiles.

## Screenshots

Binary and bundle workers can ship store screenshots from
`<worker>/screenshots/screenshots.yaml` (see
[`../sops/screenshots.md`](../sops/screenshots.md)). The compiler checks them
against the Registry's limits and writes the descriptor's top-level
`screenshots` (never in `registry_projection`); the build uploads each image to
`build-<source_sha>` as `<worker>-screenshot-<name>` and lists it in the build
manifest's `screenshots`; Release Control registers them as `kind: screenshot`
assets of the version's Registry release and names them in `POST /publish`.
The field's absence lets a version inherit the previous version's list, and
`[]` clears it.

| `artifact.kind` | Screenshots |
|---|---|
| `rust-binary`, `go-binary` | yes (Registry release type `binary`) |
| `javascript-bundle`, `python-bundle` | yes (Registry release type `bundle`) |
| `oci-image` | only `[]`: an image version has no Registry release to host them |

Whatever its kind, a worker the Registry hides (`iii`: `GET /w/iii` answers
404) may only list `[]`, since its screenshots could never be read back.
