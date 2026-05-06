# provider-cli

Wraps installed coding CLIs (claude, codex, opencode, openclaw, hermes, pi,
gemini, cursor-agent) as iii provider functions under `provider::cli::*`.

## Installation

```bash
iii worker add provider-cli
```

The install resolver pulls `shell-bash` (declared in `iii.worker.yaml`) so
`shell::bash::which` and `shell::bash::exec` are available on the bus.

## Run

```bash
iii-provider-cli
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `shell-bash` | `^0.1.0` | Calls `shell::bash::which` to probe each CLI and `shell::bash::exec` to drive it. |

## Build

```bash
cargo build --release
```
