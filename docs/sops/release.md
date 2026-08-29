# Release train operations

Release Control is the exclusive operator interface. Do not dispatch release
workflows by hand, rerun a mutating Actions run, move Registry channels
directly, or retag GHCR outside a Release Control recovery operation.

## Before starting

The selected source commit must already contain the exact candidate version in
the worker's `source.package_manifest`. The root catalog and sanitized release
defaults must also be committed at that SHA. Prepare never bumps, commits, or
pushes source.

[`release-descriptor-index.yml`](../../.github/workflows/release-descriptor-index.yml)
compiles every worker at the source SHA with the immutable iii compiler pin.
Its artifact contains `release-descriptor-index.json` and exact
`descriptors/<worker>.json` files. Release Control verifies the workflow,
source SHA, compiler SHA, artifact and descriptor digest before planning.

## Sequence

1. `release-prepare.yml` authorizes the dispatch, verifies descriptor identity,
   builds one job per target, boots the prepared adapter, snapshots its typed
   interface, and uploads byte-unchanged inputs plus `release-evidence.json`
   with the SHA-256 and size of every descriptor, interface and build artifact.
2. `release-candidate-publish.yml` publishes GitHub assets, a digest-pinned OCI
   image when applicable, the immutable Registry package, and CASes `next`.
3. `release-candidate-smoke.yml` resolves the exact descriptor and OCI digest,
   boots `package://<worker>@next` through Worker Compose, and compares the live
   typed interface with the prepared snapshot without reading the source tree.
4. `release-stable-publish.yml` CASes the same candidate version to `latest`.
5. `release-image-alias.yml` moves the requested OCI alias by immutable digest.
6. `release-finalize.yml` confirms the promoted Registry identity.
7. `release-verify.yml` verifies GitHub, Registry and optional GHCR surfaces.

Every entrypoint authorizes with GitHub OIDC audience
`release-control-workers`. It uploads
`release-result-<candidate-id>-<step-id>-attempt-<run-attempt>` containing the
single file `release-result.json`, then posts those exact bytes to Release
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
