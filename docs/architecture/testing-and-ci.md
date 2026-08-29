# Testing and CI

How pull requests are gated for workers in this monorepo.

**Sources of truth:** [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml),
[`.github/scripts/discover_changed_workers.py`](../../.github/scripts/discover_changed_workers.py),
[`.github/scripts/validate_worker.py`](../../.github/scripts/validate_worker.py).

## Discovery

`discover` runs `discover_changed_workers.py` comparing a PR to its base or a
trusted `main` push to the previous revision.
A directory is a **worker** when it is owned by a `source.path` entry in the
private `.release/workers.yaml` build catalog. Publishable worker directories
also retain `iii.worker.yaml` as the public `iii worker` contract.

Outputs:

| Key | Meaning |
|---|---|
| `all` | Every changed worker folder |
| `source_changed` | Workers with non-metadata file changes |
| `rust` / `node` / `python` | Language buckets from `.release/workers.yaml` artifact kinds |

**Harness fan-out:** when `harness/` changes, in-repo deps listed in
`harness/worker-compose.yaml` `dependencies` join the rust matrix (version-bump
gates still apply only to workers the PR author edited).

**Metadata-only PRs:** if a worker's only changes match
`.release/workers.yaml`, `iii.worker.yaml`, `README.md`, `Cargo.toml`, `Cargo.lock`, `AGENTS*.md`,
version/tests/README gates downgrade to GitHub notices.

## pr-checks (per changed worker)

[`validate_worker.py`](../../.github/scripts/validate_worker.py):

1. `README.md` exists and is non-empty
2. the `.release/workers.yaml` entry parses with required private build fields
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

**Opt-out:** `validation.interface: skipped` in the release catalog, mirrored by
`interface_smoke: false` in `iii.worker.yaml` (for example `lsp`).

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

## Script tests

`.github/scripts/tests/` — pytest for release/discovery/workflow helpers. Runs
on every PR and trusted `main` push.

## Related

- New worker expectations: [`../sops/new-worker.md`](../sops/new-worker.md) §4–§5
- Release interface collection: [`../sops/release.md`](../sops/release.md)
