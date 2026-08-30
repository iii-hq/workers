# Release train operations

Release Control is the exclusive operator interface. Do not dispatch release
workflows by hand, rerun a mutating Actions run, move Registry channels
directly, or retag GHCR outside a Release Control recovery operation.

## Before starting

The selected source commit must contain the worker's `.deploy/workers.yaml`
entry and public `iii.worker.yaml`. The package-manifest version is recorded as
source metadata; Release Control owns the independent published RC and stable
versions. Prepare never bumps, commits, or pushes source.

[`deploy-descriptor-index.yml`](../../.github/workflows/deploy-descriptor-index.yml)
compiles every worker at the source SHA with the Workers-owned compiler.
Its artifact contains `deployment-descriptor-index.json` and exact
`descriptors/<worker>.json` files. Release Control verifies the workflow,
source SHA, compiler commit/digest, artifact and descriptor digest before
planning.

## Sequence

1. `deploy-prepare.yml` authorizes the dispatch, verifies descriptor identity,
   builds one job per target, boots the prepared adapter, snapshots its typed
   interface, and uploads byte-unchanged inputs plus `deployment-evidence.json`
   with the SHA-256 and size of every descriptor, interface and build artifact.
2. `deploy-candidate-publish.yml` publishes GitHub assets, a digest-pinned OCI
   image when applicable, the immutable Registry candidate, and assigns `next`.
3. `deploy-stable-publish.yml` publishes the stable identity from the prepared
   bytes without rebuilding and CASes `next` from the promoted RC to stable.
4. `deploy-image-alias.yml` moves the requested OCI alias by immutable digest.
5. `deploy-finalize.yml` CASes `latest` only after `next` resolves to stable.
6. `deploy-verify.yml` verifies GitHub, Registry and optional GHCR surfaces.

Every entrypoint authorizes with GitHub OIDC audience
`release-control-workers`. It uploads
`deployment-result-<candidate-id>-<step-id>-attempt-<run-attempt>` containing the
single file `deployment-result.json`, then posts those exact bytes to Release
Control with their SHA-256 header.

## Recovery

Use the failed operation's recovery action in Release Control. A recovery gets
a new operation/step/nonce and reuses immutable descriptor and prepared
artifacts. Results report effects as `unknown` when the workflow cannot prove a
mutation completed, allowing reconciliation without pretending success.

If either physical macOS runner pool cannot schedule three independent jobs,
stop the release and fix external capacity first. The diagnostic
[`macos-runner-capacity.yml`](../../.github/workflows/macos-runner-capacity.yml)
tests both Intel and Apple Silicon gates; this repository does not provision
EC2 Mac hosts.
