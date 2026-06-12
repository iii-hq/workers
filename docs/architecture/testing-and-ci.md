# Testing and CI

How pull requests are gated for workers in this monorepo.

**Sources of truth:** [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml),
[`.github/scripts/discover_changed_workers.py`](../../.github/scripts/discover_changed_workers.py),
[`.github/scripts/validate_worker.py`](../../.github/scripts/validate_worker.py).

## Discovery

`discover` job runs `discover_changed_workers.py` comparing the PR to its base.
A directory is a **worker** when it contains `iii.worker.yaml` at its root.
`docs/` is not discovered (no manifest).

Outputs:

| Key | Meaning |
|---|---|
| `all` | Every changed worker folder |
| `source_changed` | Workers with non-metadata file changes |
| `rust` / `node` / `python` | Language buckets from `iii.worker.yaml` |
| `vscode_changed` | `iii-lsp-vscode/` changed (special-case job) |

**Harness fan-out:** when `harness/` changes, in-repo deps listed in
`harness/iii.worker.yaml` `dependencies` join the rust matrix (version-bump
gates still apply only to workers the PR author edited).

**Metadata-only PRs:** if a worker's only changes match
`iii.worker.yaml`, `README.md`, `Cargo.toml`, `Cargo.lock`, `AGENTS*.md`,
version/tests/README gates downgrade to GitHub notices.

## pr-checks (per changed worker)

[`validate_worker.py`](../../.github/scripts/validate_worker.py):

1. `README.md` exists and is non-empty
2. `iii.worker.yaml` parses with required fields
3. Manifest version ≥ version on base branch
4. `tests/` exists and is non-empty
5. Bootstrap workers (`shell`, `iii-directory`): `skills/SKILL.md` present,
   non-empty, ≤ 256 KiB

## Language jobs

| Language | Lint | Test |
|---|---|---|
| Rust | `cargo fmt --check`, `clippy -D warnings` | `cargo test --all-features` |
| Node | `biome ci` | `npm test` (if `tests/` exists) |
| Python | `ruff check`, `ruff format --check` | `pytest` (if `tests/` exists) |

Workers with `web/package.json` (e.g. `console`) pre-build the SPA before cargo
in both `rust` and `interface-smoke` jobs.

## Interface boot smoke (Rust)

**Why it exists:** release publish boots the worker on a clean runner with no
`data/` directory. A worker can compile and pass unit tests yet crash at publish
when SQLite parent dirs or sidecars are missing (#104 / `database/v0.2.6`).

**Flow:**

1. `cargo build` (default features — same as release binary)
2. Install `iii` CLI + start engine
3. Start worker from `./target/debug/<bin>` (with `--config config.collect.yaml` when shipped)
4. `collect_worker_interface.py` — 120 s wait, assert non-empty interface

**Opt-out:** `interface_smoke: false` in `iii.worker.yaml` (e.g. `iii-lsp`).

## Dedicated e2e workflows

Some workers have harness-level e2e beyond unit tests:

| Workflow | Worker |
|---|---|
| `shell-e2e.yml` | `shell` |
| `database-e2e.yml` | `database` |
| `storage-e2e.yml` | `storage` |

Add a dedicated workflow when integration with the full harness stack is
release-blocking and too slow for the per-PR matrix.

## Script tests

`.github/scripts/tests/` — pytest for release/discovery helpers. Runs on every PR.

## Related

- New worker expectations: [`../sops/new-worker.md`](../sops/new-worker.md) §4–§5
- Release interface collection: [`../sops/release.md`](../sops/release.md)
