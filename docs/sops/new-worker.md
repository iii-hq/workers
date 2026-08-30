# Add a worker

Use this checklist for every first-party worker. Rust daemon implementation
details are in [`binary-worker.md`](binary-worker.md).

## Identity and files

- Choose a root folder and catalog slug matching
  `^[a-z0-9][a-z0-9_-]*$`.
- Add the package manifest that owns the version: `Cargo.toml`, `package.json`,
  or `pyproject.toml`.
- Add a consumer-facing `README.md` and a non-empty `tests/` directory.
- Add `iii.worker.yaml` for local development, scaffolding and `iii worker`.
- Add an entry to the private
  [`.deploy/workers.yaml`](../../.deploy/workers.yaml) build catalog.

The private entry contains exactly `source`, `artifact`, `validation`, and
`publish`. Public runtime and Registry metadata stay in `iii.worker.yaml`.
See [`worker-compose.md`](../architecture/worker-compose.md) for the boundary.
Release Control reads only the compiled descriptor index and never either
source document.

## Registry metadata

For a first-party package, set private `publish: true` and put its non-empty
license, description, dependency ranges, configuration, and discovery tags in
the public manifest. Fixtures use private `publish: false` and are never
deployment targets.

Never put API keys, tokens, `III_*` connection settings, or mutable external
references in public defaults; the compiler rejects them before producing the
immutable Registry projection.

The PR interface boot check is enabled by default. Set
`interface_smoke: false` in `iii.worker.yaml` only for a worker that cannot
expose a collectable interface, such as a stdio-only process. This setting does
not affect deployment or publication.

## Local checks

```bash
python3 .github/scripts/validate_worker.py \
  --worker <slug> --base-ref origin/main --source-changed '["<slug>"]'

python3 .github/scripts/deployment_compiler.py compile-index \
  --source-sha "$(git rev-parse HEAD)" \
  --compiler-repository iii-hq/workers \
  --compiler-commit "$(git rev-parse HEAD)" \
  --output-dir /tmp/deployment-descriptor-index
```

Run the language suite that CI will select:

- Rust: `cargo fmt --all -- --check`, clippy, and locked tests.
- JavaScript: Biome and package tests.
- Python: Ruff and pytest.

Handlers published as interface metadata must have typed request and response
schemas. Add a focused schema/catalog test for the worker and a dedicated E2E
workflow when startup, sidecars, or external protocol behavior needs coverage.

## Release readiness

Commit the source metadata in `source.package_manifest`. The descriptor index
workflow records its version at the merged source SHA, while Release Control
selects the exact deployment target version independently. Prepare never edits
the manifest. Ensure each declared
bundle file exists, every Rust target is supported, and OCI Docker inputs are
fully pinned before requesting a release.

New release policy belongs in Release Control. Do not add another tag-triggered
or repository-local release orchestrator.
