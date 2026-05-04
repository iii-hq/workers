# shell-filesystem

Sandboxed filesystem operations on the iii bus under `shell::fs::*`.

## Installation

```bash
iii worker add shell-filesystem
```

## Run

```bash
iii-shell-filesystem --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

`shell::fs::read`, `shell::fs::write`, `shell::fs::list`, `shell::fs::stat`,
`shell::fs::glob`, plus binary read/write helpers (base64-encoded payloads).

## Build

```bash
cargo build --release
```
