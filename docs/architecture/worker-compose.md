# Public Compose and private release metadata

`worker-compose.yaml` is a public iii Compose document. It describes a running
namespace and its containers; it is not a worker catalog and must not contain
top-level `workers` or `stacks` mappings.

```yaml
namespace: harness
containers:
  state:
    worker: package://state
    version: next
    config_override: {}
  harness:
    worker: path://.
    start_after: [state]
    environment:
      RUST_LOG: info
    scripts:
      run: cargo run --locked --bin harness
```

The current public fields include `namespace`, `containers`, `worker`,
`version`, `start_after`, `config_override`, `environment`, and `scripts`.
Repository Compose files, generated deployment-verification files, and Harness E2E
fixtures all use this same shape.

## Release-only catalog

[`.deploy/workers.yaml`](../../.deploy/workers.yaml) is private to this
repository. It contains build and validation policy that does not belong in
the public manifest or Compose contract:

```yaml
workers:
  session-manager:
    source:
      path: session-manager
      package_manifest: Cargo.toml
    artifact:
      kind: rust-binary
      binary: session-manager
      toolchain: {name: rust, version: 1.97.1}
      targets:
        - x86_64-apple-darwin
        - aarch64-apple-darwin
        - x86_64-unknown-linux-gnu
    validation:
      interface: required
    publish: true
```

Each entry contains exactly `source`, `artifact`, `validation`, and `publish`.
The worker identity is the mapping key. The selected package-manifest version
is metadata; Release Control selects the exact deployment target version.

Public runtime and Registry metadata remain in `<worker>/iii.worker.yaml`:
identity, deploy kind, package manifest, binary name, description, license,
tags, semver dependencies, non-secret configuration, resources, environment,
and scripts. `iii.worker.yaml` remains supported by local development,
scaffolding, `iii worker`, and package consumers.

## Compilation boundary

The repository-owned
[`deployment_compiler.py`](../../.github/scripts/deployment_compiler.py) joins the
private release entry, public manifest, package manifest, and source SHA once.
It emits a `deployment-descriptor.json` containing input digests, independent
build units, runtime, and a projection onto the current Registry API.

Prepare and every later phase consume only that immutable descriptor and the
prepared artifacts. Release Control consumes only the descriptor index and
selected descriptor. Neither rereads `.deploy/workers.yaml`,
`iii.worker.yaml`, or a package manifest.

Bundles declare an explicit file allowlist and reject tests, documentation,
caches, `node_modules`, and traversal. OCI workers declare `linux/amd64` and
`linux/arm64` unless an explicit exception is accepted. Public defaults reject
secrets and `III_*` connection values.
