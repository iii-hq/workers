# Rust binary worker

This guide covers a first-party Rust daemon published as
`artifact.kind: rust-binary`. Use [`new-worker.md`](new-worker.md) for the
cross-cutting checklist.

## Catalog entry

Keep the public worker contract in `<worker>/iii.worker.yaml` for local
installation and development:

```yaml
iii: v1
name: <worker>
language: rust
deploy: binary
manifest: Cargo.toml
bin: <worker>
license: Apache-2.0
tags: [<discovery-tag>]
```

Add its release-only build metadata to
[`.deploy/workers.yaml`](../../.deploy/workers.yaml):

```yaml
workers:
  <worker>:
    source:
      path: <worker>
      package_manifest: Cargo.toml
    artifact:
      kind: rust-binary
      binary: <worker>
      targets:
        - x86_64-apple-darwin
        - aarch64-apple-darwin
        - x86_64-unknown-linux-gnu
        - x86_64-unknown-linux-musl
        - aarch64-unknown-linux-gnu
        - armv7-unknown-linux-gnueabihf
    validation:
      interface: required
    publish: true
```

The slug, folder, Cargo package, binary, runtime executable and registered
worker identity should match unless an existing package has an explicitly
tested exception.

## Crate shape

Each worker is an isolated Cargo workspace with a locked dependency graph:

```toml
[workspace]

[package]
name = "<worker>"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "<worker>"
path = "src/main.rs"
```

Pin the repository's supported `iii-sdk` line exactly. Register typed request
and response structs (`serde` plus `schemars`) for every function and trigger.
The process must connect to the configured iii endpoint, remain alive while it
serves requests, and shut down cleanly on SIGINT/SIGTERM.

Keep runtime configuration operator-facing and secret-free in the release
descriptor. If Registry consumers need defaults, add a sanitized release
defaults file and reference it with `registry.config.defaults_file`.

## Tests

At minimum, cover:

- configuration parsing and invalid values;
- function/trigger catalog identity;
- typed request and response schemas;
- handler success and error behavior;
- startup and clean shutdown where practical.

Before opening a PR:

```bash
cd <worker>
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

CI builds changed Rust workers and checks interface registration unless
`iii.worker.yaml` sets `registry_interface: false`. Release prepare also boots one
immutable Linux artifact in an isolated engine to capture Registry metadata.
That capture invokes no worker function or external backend and is not a
deployment smoke test.

## Release behavior

The package version in `Cargo.toml` is informational and does not select the
deployment target version. The descriptor compiler emits one build
unit per declared target. Each unit builds independently and uploads a
deterministic `<binary>-<target>.tar.gz` plus checksum; prepare later assembles
those files without changing descriptor bytes.

The Registry request maps every declared target to its immutable GitHub Release
URL and SHA-256. Publishing moves the exact target version to the requested
channel; a `latest` publication advances the `next` floor first when needed.
There is no rebuild.
