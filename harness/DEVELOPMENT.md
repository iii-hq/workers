# Harness local development

This guide explains how to run the Harness stack from the local source tree.

## Requirements

Install these tools before you start:

- Rust. The repository selects the version in [`rust-toolchain.toml`](../rust-toolchain.toml).
- The `iii` CLI version `0.23.0-rc.4` or a compatible build with managed
  Compose engine support.

Check the required commands:

```bash
iii --version
cargo --version
```

## Start the Harness stack

The Harness [`worker-compose.yaml`](worker-compose.yaml) file runs the Harness
and its required workers from their directories in this repository. It also
starts and configures the iii engine. From the repository root, run:

```bash
cd harness
iii compose up
```

The managed engine listens on `ws://127.0.0.1:49134`. Do not start a separate
engine or pass `--engine` with this Compose file.

Set `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` in the Compose terminal when you
need the related provider. Keep these values in your local shell or secret
manager. Do not add them to the repository.

Compose starts each Rust worker with `cargo run`. The local startup flow is:

```text
iii compose up
      |
      +--> managed iii engine
      |
      +--> cargo run for each worker
                    |
                    v
          worker connects to the engine
```

The first run can take several minutes because Cargo must download and compile
each worker. Compose allows up to 15 minutes for each worker to register. Later
runs normally reuse the Cargo cache.

Restart Compose after a source change to rebuild and restart the workers.

Press `Ctrl-C` in the Compose terminal to stop the workers and the managed
engine.

## Run checks

Run the checks from the worker directory that you changed:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
