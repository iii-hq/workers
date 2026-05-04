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

## Registered functions

`shell::bash::run`, `shell::bash::which`, `shell::bash::detect_clis`.

## Engine functions consumed

`sandbox::exec` — provided by the iii engine `iii-exec` builtin.

## Build

```bash
cargo build --release
```
