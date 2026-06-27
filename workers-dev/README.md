# workers-dev

Local dev orchestrator for all iii workers in this repo. Starts Rust binary workers via `cargo run`, shows a TUI dashboard with live engine connection status, and supports dependency-aware restarts.

## Prerequisites

1. **`iii` CLI** on `PATH`.
2. **Running iii engine** (this tool does not start the engine):

```bash
iii -c harness/engine.config.yaml
```

3. **Provider API keys** for the harness stack:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
```

## Install

```bash
cargo install --path workers-dev
```

## Worker groups

Workers are **discovered automatically** from top-level `*/iii.worker.yaml` in this repo.

| Group | Workers | Started by |
|-------|---------|------------|
| **harness stack** | `session-manager`, `llm-router`, `context-manager`, `provider-anthropic`, `provider-openai`, `approval-gate`, `harness` | `workers-dev up`, `Ctrl+u` in TUI, `workers-dev start` |
| **other** | All remaining repo workers (e.g. `telegram-bot`, `shell`, `console`, …) | `workers-dev start <name>`, `workers-dev start --all`, `Ctrl+a` in TUI |

Only **Rust `deploy: binary`** workers can be started with `cargo run`. Node/bundle workers show as `(iii worker add)` — install them via the iii registry instead.

A worker can read **Process: stopped** while **Engine: connected**. That means it's connected to the engine but was not started by this `workers-dev` (e.g. started elsewhere, or via `iii worker add`) — `workers-dev` only tracks processes it spawned itself.

## Usage

```bash
workers-dev up                    # start harness stack + TUI
workers-dev                       # TUI only
workers-dev start                 # harness stack (CLI, waits for connect)
workers-dev start --all           # every discovered Rust worker
workers-dev start telegram-bot    # one worker (+ deps)
workers-dev restart llm-router    # rebuild + restart dependents
workers-dev logs harness -f
workers-dev status
```

Global flags: `--repo`, `--url`, `--port`, `--release`, `--config workers-dev.yaml`, `--stop-on-exit`, `--color auto|always|never`.

Environment: `WORKERS_DEV_REPO` overrides repo auto-detection. Set `NO_COLOR` to disable colors (also respected when `--color auto`).

## Colors

Semantic colors are applied at render time (logs stay plain text in the ring buffer):

- **Worker table**: green/yellow/red/gray for status, process, and engine columns
- **Log pane / `workers-dev logs -f`**: cargo progress (yellow), build done (cyan), errors (red), tracing levels (INFO/WARN/DEBUG)

Use `--color never` or `NO_COLOR=1` to force plain output. Default `--color auto` enables colors on a TTY and disables them when stdout is piped.

## TUI keys

| Key | Action |
|-----|--------|
| `↑`/`↓` | Select worker (skips group headers) |
| `s` | Start selected worker |
| `x` | Stop selected worker |
| `r` | Restart selected worker + dependents |
| `l` | Follow logs |
| `Ctrl+u` | Start harness stack |
| `Ctrl+a` | Start all managed Rust workers |
| `q` | Quit |

The log pane shows the last 20 lines for the **selected worker only** (cargo build output + worker logs). Lines are sanitized (no ANSI, no `\r` overwrite garbage).

## Config (`workers-dev.yaml`)

```yaml
repo: /path/to/workers
engine_url: ws://127.0.0.1:49134
release: false
workers:          # optional override; default = all discovered
  - session-manager
  - harness
harness_stack:    # optional override; default = harness stack subset
  - session-manager
  - llm-router
  - harness
color: auto   # auto | always | never (respects NO_COLOR)
```

## Troubleshooting

**Garbled log lines in the dashboard**

Usually caused by cargo `\r` progress lines or ANSI color codes. Current versions normalize both. Reinstall: `cargo install --path workers-dev`.

**Engine not reachable**

Start the engine: `iii -c harness/engine.config.yaml`

**Non-Rust worker won't start**

Expected — use `iii worker add <name>` for JavaScript/bundle workers.
