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

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Defaults:

```yaml
max_inline_bytes: 262144
```

`SHELL_FILESYSTEM_MAX_INLINE_BYTES` overrides the config file for direct
runtime overrides.

## Registered functions

`shell::filesystem::read`, `shell::filesystem::write`,
`shell::filesystem::ls`, `shell::filesystem::stat`,
`shell::filesystem::grep`, `shell::filesystem::sed`,
`shell::filesystem::edit`, `shell::filesystem::rm`,
`shell::filesystem::mv`, `shell::filesystem::mkdir`, and
`shell::filesystem::chmod`.

## Build

```bash
cargo build --release
```
