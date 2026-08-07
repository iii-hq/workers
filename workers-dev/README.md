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
| **current stack** (default: `harness`) | The stack's roots **plus everything they transitively depend on**, derived live from each worker's `iii.worker.yaml` dependencies. The built-in `harness` stack's roots are `session-manager`, `llm-router`, `context-manager`, `provider-anthropic`, `provider-openai`, `approval-gate`, `harness`; define more stacks (or override `harness`'s roots) in `workers-dev.yaml` or from the TUI (`Space` + `n`) — see Config below | `workers-dev up` / bare `workers-dev start` start the `default_stack` (missing deps are pulled in, connected ones left alone); `Ctrl+u` in the TUI starts it directly, or opens a stack picker when more than one stack is defined |
| **other** | All remaining repo workers (e.g. `telegram-bot`, `console`, …) | `workers-dev start <name>`, `workers-dev start --all`, `Ctrl+a` in TUI |

Press `d` on any worker in the TUI to see its direct dependencies and its transitive dependents (the `r` restart blast radius), each with live status.

Only **Rust `deploy: binary`** workers can be started with `cargo run`. Non-Rust (Node/bundle) workers show **Process: external** — install them via the iii registry (`iii worker add`) instead.

A worker connected to the engine but not started by this `workers-dev` shows **Process: elsewhere** beside **Engine: connected** (`workers-dev` only tracks processes it spawned itself). Non-Rust workers installed via `iii worker add` show **Process: external**.

## Usage

```bash
workers-dev up                    # start default stack + TUI
workers-dev                       # TUI only
workers-dev start                 # default stack (CLI, waits for connect)
workers-dev start --all           # every discovered Rust worker
workers-dev start telegram-bot    # one worker (+ missing deps)
workers-dev restart llm-router    # rebuild + restart dependents
workers-dev logs harness -f
workers-dev status
```

Stacks: define named stacks in `workers-dev.yaml`, or create one from the TUI
with `Space` + `n` (see Config below). `up` and bare `start` start the
`default_stack`; in the TUI, `Ctrl+u` opens a stack picker (Enter = switch the
dashboard's current stack + start it) when more than one stack is defined, and
starts the only stack directly otherwise.

Starting a worker (CLI `start <name>` or `s` in the TUI) pulls in its dependencies, but a dependency **already connected to the engine is left running as-is** — no rebuild, no restart, no duplicate spawn. Explicitly requested workers always (re)start; use `restart` when a dependency itself needs a rebuild. The group commands count every member as explicitly requested: `up`, bare `start`, and `Ctrl+u` always restart the whole stack, `start --all` and `Ctrl+a` every managed Rust worker.

Global flags: `--repo`, `--url`, `--port`, `--release`, `--config workers-dev.yaml`, `--stop-on-exit`, `--color auto|always|never`, `--ui-watch`.

Environment: `WORKERS_DEV_REPO` overrides repo auto-detection. Set `NO_COLOR` to disable colors (also respected when `--color auto`).

## Injectable-UI watcher mode

Workers that ship injectable console UI (a `ui/package.json` — see
`docs/sops/injectable-console-ui.md`) can run in the SOP's dev loop straight
from workers-dev. When watch is on for a worker, starting it:

1. spawns **`pnpm watch`** in `<worker>/ui/` (esbuild `--watch` → `dist/`),
   stopped with the worker, output in the same log pane tagged `[ui]`;
2. sets **`III_<WORKER>_UI_WATCH=1`** on the worker process, arming the
   `iii-console-ui` crate's poller — every rebuild re-registers the changed
   asset and open console tabs hot-swap it in place.

Enable it globally with `--ui-watch` (or `ui_watch: true` in
`workers-dev.yaml`), or per worker with the **`w`** key in the TUI — a
running worker restarts so the env var takes effect; a stopped one picks it
up on next start. The dashboard's **UI** column shows `—` (no ui project),
`ui` (ships UI, watch off), or `watch` (watcher mode on).

## Colors

Semantic colors are applied at render time (logs stay plain text in the ring buffer):

- **Worker table**: green/yellow/red/gray for status, process, and engine columns
- **Log pane / `workers-dev logs -f`**: cargo progress (yellow), build done (cyan), errors (red), tracing levels (INFO/WARN/DEBUG)

Use `--color never` or `NO_COLOR=1` to force plain output. Default `--color auto` enables colors on a TTY and disables them when stdout is piped.

## TUI keys

| Key | Action |
|-----|--------|
| `↑`/`↓` (or `k`/`j`) | Select worker (skips group headers) |
| `g`/`G` (or `Home`/`End`) | Jump to the first / last worker |
| `Space` | Mark the selected worker for a new stack |
| `n` | Name and save a new stack from the marked workers |
| `s` | Start selected worker |
| `x` | Stop selected worker |
| `r` | Restart selected worker + dependents (confirm lists the blast radius with live status) |
| `w` | Toggle injectable-UI watcher mode for the selected worker (restarts it when running) |
| `d` | Show selected worker's dependencies + dependents with live status |
| `f` | Toggle live-follow of the selected worker's logs |
| `PgUp`/`PgDn` | Scroll the log pane (pauses follow; resumes at the bottom) |
| `+`/`-` | Resize the log pane (drags the divider in two columns, the height when stacked) |
| `/` | Filter workers by name (Enter applies, Esc clears) |
| `e` | Start the iii engine (`iii -c harness/engine.config.yaml`) |
| `Ctrl+u` | Start stack (picker when several stacks are defined; Enter = switch + start) (in the picker: `x` delete a stack, `*` make it the default) |
| `Ctrl+a` | Start all managed Rust workers |
| `?` | Toggle the key-reference overlay |
| `q` | Quit |

On a wide terminal the dashboard is a two-column **master/detail** layout: the worker list on the left (sized to fit its columns), the selected worker's logs filling the rest on the right, with `+`/`-` dragging the divider between them. Below ~100 columns the two panes stack vertically instead, and `+`/`-` trade height.

The header shows the repo's current git branch (`⎇ feat/my-branch`, refreshed live; detached HEAD shows as `@<short-hash>`) so side-by-side instances on different worktrees or checkouts are easy to tell apart — the terminal/tmux pane title is set to `workers-dev ⎇ <branch>` too — plus an at-a-glance health summary (`●` connected, `◐` compiling, `✗` crashed, `○` stopped). When an engine status query fails the header flags `⚠ unreachable` and gains a line with the remedy (`press e to start the engine`) and the underlying error. The worker list's title shows the selection position (`Workers 3/48`). Each group's header row reads `── stack:<name> (N) ──` for the current stack and `── other (N) ──` for everything else, where N is the post-filter worker count. The log pane shows the **selected worker only**, scrollable through the full ring buffer, following the live tail by default. Crashed workers show their exit code inline. Lines are sanitized (no ANSI, no `\r` overwrite garbage).

## Config (`workers-dev.yaml`)

Auto-loaded from the repo root when present; `--config <path>` overrides.

```yaml
repo: /path/to/workers
engine_url: ws://127.0.0.1:49134
release: false
workers:          # optional override; default = all discovered
  - session-manager
  - llm-router
  - harness
  - console
stacks:           # optional named stacks; values are stack ROOTS —
  console:        # the group shown/started is roots + transitive deps.
    - console
    - session-manager
  harness:        # overrides the built-in harness stack's roots
    - session-manager
    - llm-router
    - harness
default_stack: console   # started by `up` / bare `start`; default: harness
color: auto   # auto | always | never (respects NO_COLOR)
ui_watch: false   # start injectable-UI workers in watcher mode (pnpm watch + III_<WORKER>_UI_WATCH=1)
```

The built-in `harness` stack always exists. The old `harness_stack:` key was
replaced by `stacks:` + `default_stack:` and now fails startup with a rename
hint.

Stacks can also be created from the TUI: mark workers with `Space`, press `n`,
name it, Enter. The stack is written into this file (comments and formatting
are preserved) and becomes the current stack immediately — it is *not* started;
press `Ctrl+u` when you want that. In the `Ctrl+u` picker, `x` deletes a stack
and `*` makes it the default. Deleting the default stack is refused — set
another default first.

Note: `workers-dev.yaml` is not gitignored, so the first save leaves an
untracked file in the repo root. `workers-dev` writes the file by editing the
lines it owns; if `stacks:` is written inline (`stacks: {a: [b]}`) it refuses
to edit and says so.

## Troubleshooting

**Garbled log lines in the dashboard**

Usually caused by cargo `\r` progress lines or ANSI color codes. Current versions normalize both. Reinstall: `cargo install --path workers-dev`.

**Engine not reachable**

Press `e` in the TUI, or start the engine manually: `iii -c harness/engine.config.yaml`

**Non-Rust worker won't start**

Expected — use `iii worker add <name>` for JavaScript/bundle workers.
