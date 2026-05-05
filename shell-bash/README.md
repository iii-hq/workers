# shell-bash

Sandboxed shell execution on the iii bus under `shell::bash::*`. Wraps the
iii engine `sandbox::exec` primitive — no host shell fallback.

## Installation

```bash
iii worker add shell-bash
```

## Run

```bash
iii-shell-bash --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Defaults:

```yaml
default_timeout_ms: 30000
trigger_timeout_ms: 35000
max_output_bytes: 30000
```

`SHELL_BASH_DEFAULT_TIMEOUT_MS`, `SHELL_BASH_TRIGGER_TIMEOUT_MS`, and
`SHELL_BASH_MAX_OUTPUT_BYTES` override the config file for direct runtime
overrides. Per-call `timeout_ms` still overrides `default_timeout_ms`.

## Registered functions

`shell::bash::exec`, `shell::bash::which`, `shell::bash::detect_clis`.

## Engine functions consumed

`sandbox::exec` — provided by the `iii-worker` sandbox surface
(see `iii/crates/iii-worker/src/cli/sandbox.rs`). Note: `iii-exec` is a
different engine builtin (a startup pipeline daemon) and exposes no bus
functions.

## Build

```bash
cargo build --release
```
