# Release train operations

Release Control is the exclusive operator interface. Do not dispatch release
workflows by hand, rerun a mutating Actions run, move Registry channels
directly, or retag GHCR outside a Release Control recovery operation.

## Before starting

The selected source commit must contain the worker's `.deploy/workers.yaml`
entry and public `iii.worker.yaml`. The package-manifest version is source
metadata; Release Control owns the independent exact `target_version`. Prepare
never bumps, commits, or pushes source.

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
2. `deploy-publish.yml` publishes or proves GitHub assets, the exact Registry
   version and a digest-pinned OCI image when applicable, then CASes the
   requested `next` or `latest` channel from the value captured in the plan.
   For `latest`, it first advances `next` only when the target is ahead and
   never moves `next` backwards. The OCI channel alias is updated by digest in
   this same workflow.
3. `deploy-verify.yml` verifies GitHub, Registry and optional GHCR surfaces.

Every entrypoint authorizes with GitHub OIDC audience
`release-control-workers`. It uploads
`deployment-result-<deployment-target-id>-<step-id>-attempt-<run-attempt>` containing the
single file `deployment-result.json`, then posts those exact bytes to Release
Control with their SHA-256 header.

## Recovery

Use the failed operation's recovery action in Release Control. A recovery gets
a new operation/step/nonce and reuses immutable descriptor and prepared
artifacts for the same exact target. Results report effects as `unknown` when the workflow cannot prove a
mutation completed, allowing reconciliation without pretending success.

If either physical macOS runner pool cannot schedule three independent jobs,
stop the release and fix external capacity first. The diagnostic
[`macos-runner-capacity.yml`](../../.github/workflows/macos-runner-capacity.yml)
tests both Intel and Apple Silicon gates; this repository does not provision
EC2 Mac hosts.
