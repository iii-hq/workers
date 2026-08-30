# Deployment executor cutover

`.deploy/workers.yaml` is the deployment catalog. The pinned compiler emits
`deployment-descriptor` snapshots whose package-manifest version is
informational; Release Control supplies the candidate and stable versions.

Only these Release Control-authorized entrypoints may mutate deployment state:

- `deploy-prepare.yml`
- `deploy-candidate-publish.yml`
- `deploy-stable-publish.yml`
- `deploy-image-alias.yml`
- `deploy-finalize.yml`
- `deploy-verify.yml`

Prepare builds one independent job per build unit and uploads deterministic
artifacts. Every later phase downloads those artifacts and refuses identity or
digest drift. Candidate publication creates the immutable RC and moves
`@next`. Stable publication creates the immutable stable version and moves
`@next`; finalize moves `@latest`, preserving `@next >= @latest` across partial
failure. No publication phase compiles source.

Each executor writes `deployment-result.json` once, uploads it under the
candidate/step/attempt identity, obtains a GitHub OIDC token, and sends the same
bytes to Release Control. GitHub App credentials are not available to build
shards. Effect jobs use environment-scoped Registry and container credentials.

Rust uses remote `sccache` partitioned by toolchain and target. JavaScript and
Python cache keys include lockfile, runtime and architecture. The macOS
capacity workflow proves at least three simultaneous x64 slots and three
simultaneous arm64 slots; each build unit remains a separate matrix job.

At cutover, merge this repository before enabling the new workflow allowlist.
After Release Control migration `0036`, authorize only the deployment
entrypoints and run Rust/frontend, JavaScript, Python, OCI and Harness canaries.
