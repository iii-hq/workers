# Testing and CI

How pull requests are gated for workers in this monorepo.

**Sources of truth:** [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml),
[`.github/scripts/discover_changed_workers.py`](../../.github/scripts/discover_changed_workers.py),
[`.github/scripts/validate_worker.py`](../../.github/scripts/validate_worker.py).

## Discovery

`discover` runs `discover_changed_workers.py` comparing a PR to its base or a
trusted `main` push to the previous revision.
A directory is a **worker** when it is owned by a `source.path` entry in the
private `.deploy/workers.yaml` build catalog. Publishable worker directories
also retain `iii.worker.yaml` as the public `iii worker` contract.

Outputs:

| Key | Meaning |
|---|---|
| `all` | Every changed worker folder |
| `source_changed` | Workers with non-metadata file changes |
| `rust` / `node` / `python` | Language buckets from `.deploy/workers.yaml` artifact kinds |

**Harness fan-out:** when `harness/` changes, in-repo deps listed in
`harness/worker-compose.yaml` `dependencies` join the rust matrix (version-bump
gates still apply only to workers the PR author edited).

**Metadata-only PRs:** if a worker's only changes match
`.deploy/workers.yaml`, `iii.worker.yaml`, `README.md`, `Cargo.toml`, `Cargo.lock`, `AGENTS*.md`,
version/tests/README gates downgrade to GitHub notices.

## pr-checks (per changed worker)

[`validate_worker.py`](../../.github/scripts/validate_worker.py):

1. `README.md` exists and is non-empty
2. the `.deploy/workers.yaml` entry parses with required private build fields
3. `iii.worker.yaml` remains valid and agrees with the catalog on identity,
   package manifest, deploy shape and semver dependencies
4. Package-manifest version ≥ version on base branch
5. `tests/` exists and is non-empty

Skill documentation is optional and is not part of this validation gate.

## Language jobs

| Language | Lint | Test |
|---|---|---|
| Rust | `cargo fmt --check`, `cargo clippy --locked -D warnings` | `cargo test --locked --all-features` |
| Node | `biome ci` | `npm test` (if `tests/` exists) |
| Python | `ruff check`, `ruff format --check` | `pytest` (if `tests/` exists) |

Workers with `web/package.json` (e.g. `console`) pre-build the SPA before cargo
in both `rust` and `interface-smoke` jobs.

### Rust reproducibility and caches

- [`rust-toolchain.toml`](../../rust-toolchain.toml) pins the compiler used by
  local Cargo commands and every Rust workflow. Update it deliberately after
  the replacement version passes the workflow contract and Rust test suite.
- CI compilation and tests use committed lockfiles via `--locked`.
- Pull requests restore Rust caches but do not save branch-scoped copies. The
  trusted push after merge advances caches for changed Rust workspaces.
- `rust-version` is a package-level MSRV promise, not an alias for the CI
  toolchain. Add or change it only when that package is actually tested on the
  claimed minimum compiler; the repository does not infer an MSRV from the
  pinned CI version.

## Interface boot smoke (Rust)

**Why it exists:** release publish boots the worker on a clean runner with no
`data/` directory. A worker can compile and pass unit tests yet crash at publish
when SQLite parent dirs or sidecars are missing (#104 / `database/v0.2.6`).

**Flow:**

1. `cargo build --locked` (default features — same as release binary)
2. Install `iii` CLI + start engine
3. Start worker from `./target/debug/<bin>` (with `--config config.collect.yaml` when shipped)
4. `collect_worker_interface.py` — 120 s wait, assert non-empty interface

**Opt-out:** `interface_smoke: false` in `iii.worker.yaml` (for example `lsp`).
This is a PR-only check and is not part of deployment prepare or publication.

## Dedicated e2e workflows

Some workers have harness-level e2e beyond unit tests:

| Workflow | Worker |
|---|---|
| `shell-e2e.yml` | `shell` |
| `database-e2e.yml` | `database` |
| `storage-e2e.yml` | `storage` |
| `rbac-proxy-e2e.yml` | `rbac-proxy` |

Add a dedicated workflow when integration with the full harness stack is
release-blocking and too slow for the per-PR matrix.

## Runner pools

Runner labels identify one infrastructure pool. Do not reuse a label across
GitHub-hosted and self-hosted machines: routing must remain deterministic for
cost, isolation and performance measurements.

| Label | Infrastructure | Capacity | Current use |
|---|---|---:|---|
| `ubuntu-latest` | Standard GitHub-hosted Linux | GitHub-managed | PR checks, normal E2E, Harness validation/execution shards and PR stack builds |
| `workers-ci-linux-8core` | Larger GitHub-hosted Linux | 2 concurrent | Trusted `main` Harness stack build and its manual benchmark |
| `workers-release-control-linux-2core` | Larger GitHub-hosted Linux | 8 concurrent | Reserved for a later migration of release control jobs; no workflow selects it yet |
| `workers-release-linux-8core` | Larger GitHub-hosted Linux | 8 concurrent | Linux release builds and, until migrated, release control jobs |
| `windows-latest` | Standard GitHub-hosted Windows | GitHub-managed | Windows release builds |
| `workers-release-macos-12core` | Larger GitHub-hosted Intel macOS | 3 concurrent | Intel macOS release builds |
| `workers-release-macos-arm-5core` | Larger GitHub-hosted Apple Silicon | 3 concurrent | ARM macOS release builds |
| `workers-release-macos-aws-intel` | Self-hosted Intel macOS on AWS | 3 registered | Explicit contingency capacity; no workflow selects it by default |

The `workers-ci-linux-8core` group is restricted to the Harness integration
and benchmark workflows. Release pools stay in the release-only runner group
and must not be granted to pull-request workflows. The legacy `rust`
self-hosted label is not part of the Workers routing contract.

### Harness integration recovery and benchmark

[`_harness-integration.yml`](../../.github/workflows/_harness-integration.yml)
builds the complete stack once. The trusted `main` push uses
`workers-ci-linux-8core`; PR merge refs build the same bundle on
`ubuntu-latest` because the larger-runner group accepts only selected trusted
branch/tag workflow refs. The build produces a checksummed bundle tied to the
checked-out commit and containing the freshly downloaded latest `iii @rc` plus
every compiled stack binary. Independent Integration and Playwright jobs verify
that bundle and run in parallel on `ubuntu-latest`; scenario validation also
runs there while the stack builds. Callers can override `runner` for the build
or `execution-runner` for the other jobs without changing this dependency
graph.

Use
[`harness-integration-benchmark.yml`](../../.github/workflows/harness-integration-benchmark.yml)
to compare `ubuntu-latest` with `workers-ci-linux-8core` for the single build
job while keeping execution on the standard runner:

1. Select one immutable `source-ref` SHA for the entire cohort.
2. Run ten candidate executions: seven `warm` and three `cold`. Run matched
   standard-runner baselines with both cache states on the same source SHA.
3. `warm` uses the production cache keys. Each `cold` run gets a unique key
   and cannot save it, so it neither restores nor pollutes the warm cache.
4. Record queue time and execution time from the job API, cache hits from the
   job summary, and billed minutes for the larger runner.
5. Keep the 8-core pool only when build p95 is below 10 minutes and the
   improvement over `ubuntu-latest` is at least 25%. Otherwise move the build
   back to `ubuntu-latest`; execution shards remain on that pool either way.

Initial service targets are CI queue p95 below 60 seconds, no job waiting more
than five minutes for a runner, Harness integration execution p95 below ten
minutes, release prepare p95 below ten minutes and publish p95 below four
minutes.

### Initial 8-core pilot (2026-09-01)

The first `workers-ci-linux-8core` pilot completed three cold and seven warm
executions successfully. Normal queue time was 2–3 seconds. The first cold
queue is excluded because it includes the one-time runner-group access repair.

| Cohort | Samples | p50 execution | p95 execution |
|---|---:|---:|---:|
| 8-core warm | 7 | 8m54 | 9m17 |
| 8-core cold | 3 | 17m32 | 19m11 |

A matched warm `ubuntu-latest` run took 11m11, so the 8-core warm p50 improved
the original monolithic job by about 20%, below the 25% retention threshold.
The ten 8-core jobs consumed 121 rounded billed minutes, approximately US$2.66
at the price used for the pilot. That result does not measure the current
topology: the larger runner is now billed only for one shared build, while the
two execution shards run concurrently on standard runners. Rebenchmark this
build-only topology after changes to the Harness workload or cache keys.

## Script tests

`.github/scripts/tests/` — pytest for release/discovery/workflow helpers. Runs
on every PR and trusted `main` push.

## Related

- New worker expectations: [`../sops/new-worker.md`](../sops/new-worker.md) §4–§5
- Release interface collection: [`../sops/release.md`](../sops/release.md)
