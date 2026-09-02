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

Releases are rc-first: `next` only ever receives `X.Y.Z-rc.N` candidates
(minted by the nightly window or an operator deploy), and `latest` only ever
receives the pure `X.Y.Z` minted by a `finalize_release` operation from a
tested candidate.

1. `deploy-prepare.yml` authorizes the dispatch, verifies descriptor identity,
   builds one job per **candidate-profile** target, and uploads the
   byte-unchanged descriptor, prepared inventory, and artifacts with their
   SHA-256 and size. It then captures registered functions and triggers from
   one immutable artifact and binds that interface evidence to the descriptor
   and prepared inventory (retained 90 days as the finalization fast path).
2. `deploy-publish.yml` publishes or proves GitHub assets, the exact Registry
   version and a digest-pinned OCI image when applicable, then CASes the
   requested channel from the value captured in the plan.
3. `deploy-finalize.yml` executes `finalize_release` operations exclusively:
   it re-verifies the rc bytes (from the retained artifact, or from the rc's
   immutable GitHub Release checked against Registry hashes when the artifact
   expired), builds only the supplemental **stable-profile** targets (Windows
   `x86_64-pc-windows-msvc` for opted-in workers), assembles the stable
   inventory without rebuilding anything already proved, publishes the pure
   version to the Registry **without a channel**, and then moves `next` and
   `latest` together through the Registry's transactional finalize primitive.
   It shares the `deployment-<worker>` concurrency group with publish.
4. `deploy-verify.yml` verifies GitHub, the exact Registry interface captured
   during prepare, and optional GHCR surfaces.

Interface capture is a publication-integrity step. It starts the artifact only
to observe registration against an isolated engine; it never calls a worker
function or external backend and is not a deployment smoke test.

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
