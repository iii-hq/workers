# Deployment operations

Release Control is the exclusive operator interface. This repository builds; it
never publishes. Do not move Registry channels directly, and do not retag GHCR
by hand.

## Before starting

The selected source commit must contain the worker's `.deploy/workers.yaml`
entry and public `iii.worker.yaml`. The package-manifest version is source
metadata; Release Control owns the exact version being released.

[`deploy-descriptor-index.yml`](../../.github/workflows/deploy-descriptor-index.yml)
compiles every worker at the source SHA with the Workers-owned compiler. Its
artifact contains `deployment-descriptor-index.json` and exact
`descriptors/<worker>.json` files. Release Control verifies the workflow,
source SHA, compiler commit/digest, artifact and descriptor digest before
planning.

## What a build does

[`build.yml`](../../.github/workflows/build.yml) runs for one worker and one
exact source SHA, with `correlation_id` carrying the Release Control
deployment it belongs to:

1. `resolve` downloads the descriptor checkpoint, verifies that the checkout is
   exactly `source_sha`, and emits the build matrix from the descriptor's
   `build_units`.
2. `build` runs one job per unit through
   [`_deploy-build.yml`](../../.github/workflows/_deploy-build.yml).
3. `assemble` packages the prepared inventory and captures registered functions
   and triggers by observing the artifact against an isolated engine. Interface
   capture is a publication-integrity step: it never calls a worker function or
   an external backend, and it is not a smoke test.
4. `upload` creates the shared `build-<source_sha>` prerelease if it does not
   exist and uploads each asset once. An asset already present with identical
   bytes is skipped; one present with different bytes fails the job, because
   these bytes are immutable. When the descriptor declares an image, the OCI
   index is pushed under the same `build-<source_sha>` tag, or verified if it is
   already there.
5. `manifest` writes `manifest.json` (assets with URL and SHA-256, image
   digest, descriptor, interface, evidence), attests it with
   `actions/attest-build-provenance`, and uploads it as the `build-manifest`
   artifact with 90-day retention.

Rerunning a build is safe and is the normal recovery: it converges on the same
bytes and produces the same manifest.

## What Release Control does with it

Release Control reads the manifest from the run, publishes the immutable
Registry version, moves the requested channel by compare-and-swap, creates the
versioned GitHub release pointing at the same assets, and tags the image. It
then reads every surface back and only calls the deployment converged when they
all match. Versions, channels and retries are decided there, never here.

## Renamed workers

The Registry is the source of active former names: `GET /w/{slug}` returns
`worker.aliases`, and `alias_of` identifies the canonical name when the request
used an alias. Release Control publishes a GitHub tag and release for the
canonical name and every active alias at the same version and commit. Alias
releases link to the canonical release and the existing build artifacts.
Verification must match every required name before the deployment converges.

After a rename, recover any missing historical alias releases through Release
Control. From that repository, preview the complete canonical release history:

```bash
pnpm --filter api backfill:release-aliases --workers ide,ade
```

Add `--apply` to apply the reviewed recovery. This operation creates missing
alias tags and releases using the original commits and artifact links. It does
not rebuild workers or move Registry channels. Existing conflicting tags stop
recovery instead of being overwritten; rerunning completes missing work.
Dropping an alias stops future mirroring while preserving its historical tags.

Deploy the Registry API that exposes `aliases` before deploying the Release
Control consumer. See the
[Release Control operations guide](https://github.com/iii-hq/release-control/blob/main/docs/OPERATIONS.md)
for configuration and recovery details.

## Capacity

If either physical macOS runner pool cannot schedule three independent jobs,
stop and fix external capacity first. The diagnostic
[`macos-runner-capacity.yml`](../../.github/workflows/macos-runner-capacity.yml)
tests both Intel and Apple Silicon gates; this repository does not provision
EC2 Mac hosts.
