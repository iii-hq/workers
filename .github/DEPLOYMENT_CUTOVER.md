# Deployment executor cutover

`.deploy/workers.yaml` is the deployment catalog. The pinned compiler emits
`deployment-descriptor` snapshots whose package-manifest version is
informational; Release Control supplies one exact target version and channel.

Only these Release Control-authorized entrypoints may mutate deployment state:

- `deploy-prepare.yml`
- `deploy-publish.yml`
- `deploy-verify.yml`

Prepare builds one independent job per build unit and uploads deterministic
artifacts. Every later phase downloads those artifacts and refuses identity or
digest drift. Publish creates or proves the immutable target version, then CASes
the requested `next` or `latest` channel. A latest deployment advances `next`
first only when the target is ahead, and never regresses it. OCI version images
and channel aliases are handled inside publish. No publication phase compiles
source.

Each executor writes `deployment-result.json` once, uploads it under the
deployment-target/step/attempt identity, obtains a GitHub OIDC token, and sends
the same bytes to Release Control. GitHub App credentials are not available to build
shards. Effect jobs use environment-scoped Registry and container credentials.

Rust uses remote `sccache` partitioned by toolchain and target. JavaScript and
Python cache keys include lockfile, runtime and architecture. The macOS
capacity workflow proves at least three simultaneous x64 slots and three
simultaneous arm64 slots; each build unit remains a separate matrix job.

At cutover, merge this repository before enabling the new workflow allowlist.
After Release Control migration `0036`, authorize only the deployment
entrypoints and run Rust/frontend, JavaScript, Python, OCI and Harness canaries.
