# Adding a new worker

This is a normative checklist for adding a new worker to this monorepo. Each
requirement maps directly to either a CI gate (`pr-checks`, lint, tests) or a
CD gate (Create Tag → release dispatcher). Skipping a step will block the PR
or the release.

It is written for AI agents to follow step-by-step but humans should be able
to use it just as well.

## 1. Identity

- Pick a folder name at the repo root, matching `^[a-z0-9][a-z0-9_-]*$` (same
  regex as the workers registry's `worker_name`).
- The folder name **is** the worker name. It appears in:
  - the git tag pushed by Create Tag (`<folder>/v<X.Y.Z>`)
  - `iii.worker.yaml.name`
  - the registry record at `api.workers.iii.dev`
  - the consumer install command (`iii worker add <folder>`)
- Names must be unique. If you want to ship two related workers, give them
  distinct folders (e.g. `image-resize` and `image-thumbnail`).

## 2. Required files

Each file below is checked by the `pr-checks` job in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml). Missing files fail the
PR.

| File | Required for | Purpose |
|---|---|---|
| `<worker>/README.md` | all | Non-empty. Body becomes the `readme` field on `POST /publish`. |
| `<worker>/iii.worker.yaml` | all | Declares `name`, `language`, `deploy`, `manifest` (and `bin` for Rust binaries). |
| Language manifest | all | `Cargo.toml` (Rust), `package.json` (Node), `pyproject.toml` (Python). The `version` field is the source of truth. |
| `<worker>/tests/` | all | Non-empty. Holds at least one test file the standard runner picks up. |
| `<worker>/Dockerfile` | `deploy: image` only | Listens on `III_URL`, exits cleanly on `SIGTERM`. |

### `iii.worker.yaml` shape

```yaml
iii: v1
name: my-worker          # must equal the folder name
language: rust           # rust | node | python
deploy: binary           # binary | image
manifest: Cargo.toml     # path relative to <worker>/
bin: iii-my-worker       # binary deploy only — name produced by cargo
description: One-line description shown in the registry.
```

For containers, drop `bin`.

## 3. Pick a deploy type

| Worker shape | `deploy` | What CD does |
|---|---|---|
| Rust standalone CLI/daemon | `binary` | Cross-compiles to 9 targets via [`_rust-binary.yml`](.github/workflows/_rust-binary.yml), uploads tar.gz / zip + sha256 to a GitHub Release, then publishes binary URLs via `POST /publish`. |
| Node or Python worker | `image` | Builds a multi-arch image via [`_container.yml`](.github/workflows/_container.yml), pushes to `ghcr.io/<owner>/<worker>:<version>` and `:<registry_tag>`, then publishes the image reference via `POST /publish`. |
| Rust worker with hard-to-cross-compile system deps | `image` | Same container path; ship a Rust-base `Dockerfile`. |

## 4. Linting

Lint configs live at the repo root. Per-worker overrides are allowed but
discouraged.

- **Rust** — runs `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings`. Nothing extra
  to add per worker.
- **Node** — must lint clean against [`biome.json`](biome.json). Run locally:
  `npx @biomejs/biome ci <worker>`.
- **Python** — must lint clean against [`ruff.toml`](ruff.toml). Run locally:
  `ruff check <worker> && ruff format --check <worker>`.

## 5. Tests

The standard CI runner per language. The `tests/` folder must exist and be
non-empty.

- **Rust** — `tests/integration.rs` using `#[tokio::test]`. Either call
  handler functions directly (preferred for fast tests) or boot the worker as
  a subprocess. CI runs `cargo test --all-features`.
- **Node** — `tests/*.test.ts` using [Vitest](https://vitest.dev). Add
  `vitest` to `devDependencies` and define a `test` script. Suggested:

  ```json
  "scripts": {
    "test": "vitest run"
  }
  ```

- **Python** — `tests/test_*.py` using `pytest`. Add `pytest` to a
  `[project.optional-dependencies] dev` group; CI installs with
  `pip install -e .[dev]`.

See [`image-resize/tests/integration.rs`](image-resize/tests/integration.rs),
[`todo-worker/tests/handlers.test.ts`](todo-worker/tests/handlers.test.ts),
and [`todo-worker-python/tests/test_handlers.py`](todo-worker-python/tests/test_handlers.py)
for working examples.

## 6. Pull request flow

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) will:

1. Discover which worker folders changed in this PR by reading
   `iii.worker.yaml` in each affected dir.
2. Run `pr-checks` per changed worker:
   - `README.md` exists and is non-empty.
   - `iii.worker.yaml` parses; has `name`, `language`, `deploy`, `manifest`.
   - The manifest version is **strictly greater** than the version on the
     PR's base branch.
   - `tests/` exists and is non-empty.
3. Run lint + tests for the matching language matrix.

A new worker added in a PR satisfies the "version > base" check trivially
(no base version exists yet).

## 7. Releasing

After merge to `main`:

1. Open Actions → **Create Tag**
   ([`.github/workflows/create-tag.yml`](.github/workflows/create-tag.yml)).
2. Pick `worker`, `bump` (`patch` | `minor` | `major`), and `tag`
   (`latest` | `next`).
3. The workflow bumps the manifest, commits to `main`, and pushes an
   annotated tag `<worker>/v<X.Y.Z>` whose body carries `registry-tag: <tag>`.
4. The tag push fires the [`release.yml`](.github/workflows/release.yml)
   dispatcher, which:
   - Creates the GitHub Release.
   - Routes on `deploy`: `binary` → multi-arch binaries to GH Release;
     `image` → multi-arch image to ghcr.io.
   - Calls `POST https://api.workers.iii.dev/publish` with
     `WORKERS_REGISTRY_API_KEY`.

The `tag` you picked (`latest` / `next`) becomes the registry tag attached to
this version, and is atomically moved off any previously-tagged version on
the same worker (see [`openapi.yaml`](openapi.yaml)).

## 8. Worked examples

Use these as templates:

- **Rust binary** — [`image-resize/`](image-resize/): cross-compiled CLI,
  GH Release artifacts, no Dockerfile.
- **Node container** — [`todo-worker/`](todo-worker/): `Dockerfile`,
  `iii.worker.yaml`, handlers tests via `node --test`.
- **Python container** — [`todo-worker-python/`](todo-worker-python/):
  `pyproject.toml` + ruff + `Dockerfile`, pytest under `tests/`.

## 9. Copy-paste skeleton

For an AI agent scaffolding a new worker, the minimal set of files to
materialise is:

`<worker>/iii.worker.yaml`

```yaml
iii: v1
name: <worker>
language: <rust|node|python>
deploy: <binary|image>
manifest: <Cargo.toml|package.json|pyproject.toml>
# bin: iii-<worker>            # rust binary only
description: One-line description.
```

`<worker>/README.md`

```markdown
# <worker>

One paragraph explaining what the worker does, its iii functions, and the
expected `config.yaml` shape.

## Functions

- `<worker>.<function>(input)` → `output`
```

`<worker>/tests/<smoke>.{rs,ts,py}`: at least one assertion against an
exported handler.

For `deploy: image`: `<worker>/Dockerfile` that respects `III_URL` and traps
`SIGTERM`.
