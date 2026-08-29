# Root worker catalog

[`worker-compose.yaml`](../../worker-compose.yaml) is the single source of
truth for first-party workers, release packages, runtime defaults, and named
stacks. There are no per-worker release manifests or compatibility parser.

## Top level

```yaml
workers:
  <slug>: {}
stacks:
  <stack-name>: {}
```

Worker keys are stable Registry identities. A worker entry contains exactly
`source`, `artifact`, `runtime`, `registry`, and `validation`; fields from the
retired per-directory format are rejected.

## Worker shape

```yaml
workers:
  session-manager:
    source:
      path: session-manager
      package_manifest: Cargo.toml
    artifact:
      kind: rust-binary
      binary: session-manager
      targets:
        - x86_64-apple-darwin
        - aarch64-apple-darwin
        - x86_64-unknown-linux-gnu
    runtime:
      exec: [session-manager]
      resources:
        cpu: 1
        memory_mib: 512
    registry:
      description: Durable typed conversation storage.
      license: Apache-2.0
      tags: [sessions]
      dependencies: {}
      publish: true
    validation:
      interface: required
```

`source.package_manifest` supplies the immutable package version. Non-OCI
packages require `runtime.exec`. Registry configuration may use
`config: {defaults_file: <relative-file>}`; the iii compiler resolves and
sanitizes defaults into the package descriptor so later phases never reopen
the source tree.

## Artifact variants

- `rust-binary`: `binary` plus explicit `targets`.
- `javascript-bundle` and `python-bundle`: `build_command` plus a sorted,
  explicit `include` file list. Directories and globs are rejected.
- `oci-image`: `context`, `dockerfile`, and explicit `platforms`.

Fixtures live in the same catalog with `registry.publish: false`. The current
contract gate requires 69 publishable workers and 6 fixtures.

## Stacks

Stacks only express composition and ordering. A container uses
`worker: catalog://<slug>` or `package://<slug>`, optional `start_after`, and
non-secret `config`. Runtime commands and resources belong to the worker entry.

```yaml
stacks:
  harness:
    namespace: harness
    containers:
      queue:
        worker: catalog://queue
      harness:
        worker: catalog://harness
        start_after: [queue]
```

## Compilation and validation

The pinned iii CLI is the only release descriptor compiler:

```bash
iii compose validate --file worker-compose.yaml --stack harness
iii compose descriptor --file worker-compose.yaml --worker session-manager \
  --source-sha <full-commit> --output release-descriptor.json
```

CI uses [`validate_worker.py`](../../.github/scripts/validate_worker.py) for
repository conventions and the iii compiler for the canonical package shape
and digest.
