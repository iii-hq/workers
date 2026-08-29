# Release train cutover

This document is the operator checklist for replacing the worker release
train. Release Control is the only component allowed to dispatch, retry,
reconcile, cancel, or promote a release.

## Preconditions

- Freeze new release creation, retries, and scheduled dispatches.
- Wait for active executors to finish, or mark their candidates abandoned.
- Export the Release Control database and retain the export with the old
  GitHub run and artifact identifiers.
- Verify Registry descriptor backfill has no unresolved versions.
- Verify the pinned `iii` release compiler commit and the pinned
  `release-execution` JSON Schema digest.
- Keep the new workflows outside the release runner group until the database,
  Registry, `iii`, and GitHub App changes are deployed.

## Capacity and cache

The release build jobs use the private S3 cache below. The bucket is encrypted,
versioned, blocks public access, expires cache objects after 30 days, and keeps
non-current versions for 7 days.

| Setting | Value |
| --- | --- |
| Bucket | `iii-workers-release-sccache-600627348446-us-east-1` |
| Region | `us-east-1` |
| Prefix | `sccache/<toolchain>/<target>/` |
| GitHub OIDC role | `arn:aws:iam::600627348446:role/workers-release-sccache-github` |
| macOS instance role | `workers-release-macos-intel-runner` |

Before opening release dispatches, run the capacity gate and retain evidence
that three independent jobs used each of the `workers-release-macos-12core`
and `workers-release-macos-arm-5core` labels at the same time. A matrix item is
one build unit; no item may iterate workers or targets inside a runner.

The `workers-release-hosted` runner group is restricted to an explicit
workflow allowlist. At the time this checklist was written, macOS runner IDs
799, 800, and 801 were online and idle, but the group still allowed only the
retired workflow filenames. Runner health alone is therefore not a passing
gate. After Release Control is deployed, replace the allowlist with the new
entrypoints and reusables, then dispatch `macos-runner-capacity.yml`. Its
aggregators must prove three distinct runner names and overlapping execution
intervals in both pools before release dispatching is opened.

## Cutover order

1. Deploy Registry storage and resolution for normalized package descriptors.
2. Run descriptor backfill against a restored production snapshot, then apply
   the same checked migration to production.
3. Deploy the descriptor-only `iii` runtime and release compiler.
4. Merge the Workers executors without granting their runner-group access.
5. Freeze and drain the old Release Control epoch, then export its audit data.
6. Apply the Release Control hard-cut migration and deploy the candidate,
   executor-result, event, and notification-outbox runtime.
7. Update GitHub App/webhook authorization and grant only the new workflow
   files access to the release runner groups.
8. Run Rust/frontend, JavaScript, Python, and OCI canaries end to end.
9. Open release dispatching and run the dependency-first worker wave, with the
   Harness stack last.
10. Disable the old workflow registrations and remove obsolete documentation.

### Post-merge workflow deactivation record

Do not call the GitHub API before merge. Immediately after the cutover commit
lands, verify that GitHub no longer exposes or accepts dispatches for the
retired registrations below; explicitly disable any stale registration that
the Actions UI/API still retains from repository history:

- `release.yml`, `create-tag.yml`, and `create-prerelease-tag.yml`
- `prepare-release.yml`, `publish-candidate.yml`, `candidate-smoke.yml`, and
  `publish-stable.yml`
- `promote-registry.yml`, `finalize-registry.yml`, `container-alias.yml`,
  `reconcile-github-release.yml`, and `verify-release.yml`
- `_bundle.yml`, `_container.yml`, `_rust-binary.yml`, and
  `_publish-registry.yml`

Release Control must reject those filenames before release dispatching is
reopened. Retain the deactivation audit response with the cutover evidence.

## Evidence required before the worker wave

- Publish, resolve, and lock preserve the exact package descriptor digest.
- Callback bytes equal the uploaded `release-result.json` bytes.
- Artifact fallback recovers a deliberately dropped callback.
- OCI resolve and boot use the published multi-architecture index digest.
- Slack creates one root in `C0BF0A4BPML`, orders replies behind it, updates
  terminal state, honors `Retry-After`, and can requeue a dead letter.
- Three Intel and three Apple Silicon macOS build units overlap in time.
- Warm-cache hit rate is above 80 percent for a repeated canary build.

## Rollback boundary

Before an executor produces a new external effect, rollback restores the
database snapshot, the previous deployments, the previous GitHub App/webhook
configuration, and the previous runner-group allowlist.

After any immutable publication or channel mutation, do not dispatch an old
executor. Keep the new epoch frozen, classify every effect as `absent`,
`present`, or `unknown`, and reconcile through Release Control until all
effects are known.

The S3 cache is not release state. To roll it back, remove the three `SCCACHE_*`
organization variables and the `workers-release-sccache` inline policies. The
bucket can then be retained empty for investigation or deleted after its
versioned objects are expired.
