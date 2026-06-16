# New worker onboarding

**Sources of truth:** this checklist; language scaffolds in
[`binary-worker.md`](binary-worker.md) and the todo-worker templates; CI in
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml); release in
[`release.md`](release.md). On conflict, the workflow wins — update this doc.

Cross-cutting SOP for adding **any** worker to this monorepo. For the inside
of a Rust `deploy: binary` daemon, continue with [`binary-worker.md`](binary-worker.md)
after §2.

## 1. Naming and folder rules

- **Folder name** = `iii.worker.yaml` `name` = `[package].name` (Rust) =
  `[[bin]].name` = `bin:` field.
- Pattern: `^[a-z0-9][a-z0-9_-]*$` (enforced by `TAG_RE` in
  [`.github/scripts/parse_release_tag.py`](../../.github/scripts/parse_release_tag.py)).
- Do **not** prefix with `iii-` unless the worker itself is named that way
  (e.g. `iii-directory`, `iii-lsp`).
- Git release tags use the folder name: `<worker>/vX.Y.Z`.
- **Function and trigger IDs** are `<worker>::<verb>`; multi-word segments use
  kebab-case, never snake_case (e.g. `context::count-tokens`,
  `shell::on-config-change`). See [`binary-worker.md`](binary-worker.md) §7.

## 2. Required files by deploy mode

Every worker needs a top-level folder with:

| File / dir | All workers | Notes |
|---|---|---|
| `iii.worker.yaml` | yes | Registry + CI metadata |
| Version manifest | yes | `Cargo.toml`, `package.json`, or `pyproject.toml` per `manifest:` |
| `config.yaml` | **Path A (static config):** yes — operator defaults, committed | **Path B (configuration worker):** **no** — omit; use `WorkerConfig::default()` + configuration worker; optional uncommitted local seed. See [`configuration.md`](configuration.md); `session-manager` is Path B |
| `tests/` (non-empty) | yes | See §5 |
| `README.md` | yes | Per [`worker-readme.md`](../../worker-readme.md) |

Pick a scaffold by `deploy` + `language`:

| `deploy` | `language` | Scaffold |
|---|---|---|
| `binary` | `rust` | [`binary-worker.md`](binary-worker.md) |
| `image` | `javascript` / `node` | [`todo-worker/`](../../todo-worker/) |
| `image` | `python` | [`todo-worker-python/`](../../todo-worker-python/) |
| `bundle` | `javascript` | [`harness/`](../../harness/) (monorepo bundle) |

`iii.worker.yaml` must declare valid `deploy` (`binary` | `image` | `bundle`)
and `language` (`rust` | `javascript` | `node` | `python`).

## 3. Repo registration

Add a row to the **Modules** table in [`README.md`](../../README.md). Every
shipped worker has one — kind, one-line summary, link to README or architecture.

## 4. CI expectations

On the first PR that touches the worker, [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)
runs:

| Job | What it enforces |
|---|---|
| `pr-checks` | `README.md` present; `iii.worker.yaml` valid; manifest version ≥ base; `tests/` non-empty |
| Language job | Rust: `fmt`, `clippy -D warnings`, `test`; Node: `biome ci`, `npm test`; Python: `ruff`, `pytest` |
| `interface-smoke` | Rust only: build from source, boot engine + worker, collect non-empty interface |

**Before opening the PR:**

- If the default `config.yaml` needs sidecars or local paths unavailable on a
  clean runner, ship `config.collect.yaml` (lighter boot for interface
  collection). See `storage/config.collect.yaml` and `shell/config.collect.yaml`.
- If the worker cannot be booted for interface collection (stdio server, no
  registered functions), set `interface_smoke: false` in `iii.worker.yaml`
  (e.g. `iii-lsp`). This skips interface smoke **and** registry publish at
  release time.

Metadata-only changes (`iii.worker.yaml`, `README.md`, `Cargo.toml`, `Cargo.lock`,
`AGENTS*.md`) downgrade version/tests/README gates to notices instead of hard
errors.

## 5. Tests contract

- `tests/` must exist and be non-empty — `pr-checks` enforces this on every
  changed worker.
- **Rust binary:** pattern A (integration against lib) or pattern B (Cucumber
  BDD). See [`binary-worker.md`](binary-worker.md) §9.
- **Node:** `npm test` when `tests/` exists.
- **Python:** `tests/test_*.py` runnable with `pytest`.

Dedicated e2e workflows (`shell-e2e.yml`, `database-e2e.yml`, `storage-e2e.yml`)
are added when a worker needs harness-level integration beyond unit tests.

## 6. Release wiring (one-time per worker)

Before the first release, wire the worker into the CD pipeline. Forgetting a
step can fail **silently** (e.g. tag push triggers nothing).

| # | Location | Action |
|---|---|---|
| 1 | [`.github/workflows/create-tag.yml`](../../.github/workflows/create-tag.yml) | Add worker to `inputs.worker.options` |
| 2 | [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | Add `'<worker>/v*'` to `on.push.tags` |
| 3 | [`.github/scripts/parse_publish_workers_input.py`](../../.github/scripts/parse_publish_workers_input.py) | Add to `ALLOWED_WORKERS` **only if** the worker ships `skills/` and you want out-of-band skills publishing via [`publish-worker-skills.yml`](../../.github/workflows/publish-worker-skills.yml) |
| 4 | [`.github/scripts/validate_worker.py`](../../.github/scripts/validate_worker.py) | Add to `BOOTSTRAP_WORKERS` **only if** the harness stack requires this worker's skill at boot — makes `skills/SKILL.md` a hard PR gate (currently `shell`, `iii-directory`) |

**Worked example:** `session-manager` — added to `create-tag.yml` options and
`release.yml` tag patterns. No `BOOTSTRAP_WORKERS` entry (not harness-bootstrapped).

**Fallback:** if the tag pattern is missing, run **Release** manually via
`workflow_dispatch` with the tag input until §6 row 2 is fixed.

**Known drift:** `email` ships `skills/SKILL.md` but is not in `ALLOWED_WORKERS`
today — add it when enabling out-of-band skills publish for that worker.

## 7. Agent permissions

Decide default agent-callable surfaces in [`iii-permissions.yaml`](../../iii-permissions.yaml).
Rules are first-match-wins:

- **Deny** (`'!function_id'`) for transcript-mutating, config-mutating, or
  operator-only functions. Precedent: the `session-manager` block (deny all
  `session::store::*` and write paths; reads stay at `needs_approval` default).
- **Allow** bare strings or globs for read-only introspection agents need by
  default (`engine::functions::list`, `directory::registry::workers::list`, …).

Update permissions when the worker exposes functions agents should or should not
call without approval.

## 8. Skills

Ship `skills/SKILL.md` when agents should discover **when** to use the worker
(intent, boundaries, function catalogue — not JSON schemas). Author per
[`DOCUMENTATION_GUIDELINES.md`](../../DOCUMENTATION_GUIDELINES.md).

- **Bootstrap workers** (`shell`, `iii-directory`): `skills/SKILL.md` is
  **required** (≤ 256 KiB) — the harness stack expects these skills at boot.
- **On release:** skills are auto-uploaded via `POST /w/<worker>/skills` when
  markdown is present; skipped cleanly when absent.
- **Out-of-band:** [`publish-worker-skills.yml`](../../.github/workflows/publish-worker-skills.yml)
  re-publishes skills without a version bump (worker must be in `ALLOWED_WORKERS`).

## 9. First release

Follow [`release.md`](release.md). For a brand-new worker, prefer
**registry tag `next`** on the first publish so `iii worker add` users on
`latest` are not surprised.

Recommended preflight:

```bash
# Rust binary — from the worker folder
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
./target/debug/<worker> --manifest | jq .
```

Then run **Create Tag** on `main` with the worker selected.
