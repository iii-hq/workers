# workers-dev

A dashboard and remote control for this repo's `iii compose` project. It brings
the local stack up with one command, then lets you restart, stop and read one
container at a time — the thing `iii compose --up` alone cannot do, since
`Ctrl-C` there takes the whole stack and the managed engine with it.

`workers-dev` runs no workers of its own. The stack — inventory, dependency
edges, environment, commands and the engine URL — comes from
[`harness/worker-compose.yaml`](../harness/worker-compose.yaml); every action is
a `compose::*` call. It also lists every other worker this repo ships, so
starting one is a keypress instead of a file edit.

## Requirements

- The `iii` CLI on `PATH` (0.23 or newer — it starts the engine for you).
- `cargo`, for the `cargo run` commands the compose file declares.
- `pnpm`, only if you use the injectable-UI watchers.
- The provider API keys you need, exported in the shell you launch from — see
  below.

## Install

```bash
cargo install --path workers-dev
```

## Commands

```bash
workers-dev                       # start the project and open the dashboard
workers-dev up                    # the same, spelled out
workers-dev start                 # start the project, no dashboard
workers-dev start state ade       # compose::up those containers (+ dependencies)
workers-dev start database        # a repo worker nothing declares yet: start it on demand
workers-dev stop                  # compose::stop — containers, daemon, managed engine
workers-dev stop llm-router       # compose::down that container (+ its dependents)
workers-dev restart harness       # compose::restart — that container only
workers-dev status                # one compose::status table
```

Flags: `--repo`, `--worker-dir`, `-n/--namespace`, `--color auto|always|never`,
`--ui-watch`. Environment: `WORKERS_DEV_REPO` names the repo root,
`WORKERS_DEV_WORKER_DIRS` adds worker directories (colon-separated, like `PATH`;
the `a` key writes to `harness/.workers-dev/worker-dirs` instead),
`III_ENGINE_PORT` moves the engine, `NO_COLOR` disables color.

For log history deeper than the pane keeps, the CLI that owns the logs is better
at it:

```bash
iii compose logs harness -F -n my-project -f harness/worker-compose.yaml
```

## API keys

compose builds each worker's environment from a fixed baseline (`PATH`, `HOME`,
`TERM`, and seven more) plus what the compose file declares, and drops
everything else in your shell. So a key you exported reaches nothing on its own,
and the tracked compose file names only two.

`workers-dev` forwards every variable matching `*_API_KEY` from the shell it was
launched in, through the env file that all containers already read. Export the
key, start the stack, done — including for a worker started on demand:

```bash
export DEEPSEEK_API_KEY=sk-...
workers-dev
```

Two things worth knowing:

- The keys are written to `harness/.env.workers-dev` (gitignored, mode `0600`)
  because an env file is the only channel compose re-reads per spawn. Every
  container in the project sees them, not just the one that needs them.
- **A provider's key belongs to `llm-router`, not to the provider worker.** The
  provider only declares the variable's name; `llm-router` is the process that
  calls `std::env::var` on it when it resolves a credential. Forwarding to every
  container covers this without you having to know it.

Values are forwarded as-is; a key containing a newline is skipped, because the
env-file format has one record per line and no escaping.

## What it does on launch

If no engine is listening, `workers-dev` runs

```
iii compose --namespace my-project --up --file <repo>/harness/worker-compose.yaml
```

and compose starts the engine declared under `engine:`. If an engine is already
up, it attaches with `--engine <url>` instead, so stopping compose leaves that
engine running — this is also the path that re-adopts survivors after an unclean
shutdown. If a compose daemon is already serving the namespace, nothing is
spawned at all and the dashboard simply attaches; the header says `managed` or
`attached`.

The daemon's own stdout and stderr go to `harness/.workers-dev.log`
(gitignored). `workers-dev` never signals that process: quitting the dashboard
leaves the stack running unless you ask otherwise.

## Starting other workers from the repo

The dashboard's second group, `repo`, is every worker with an `iii.worker.yaml`
that the stack does not declare — 60 of them. Pressing `s` on one writes it a
compose project of its own under **`harness/.workers-dev/`** and starts it:

```yaml
# harness/.workers-dev/database.yaml
containers:
  database:
    worker: path://../../database
    scripts:
      run: cargo run --bin database
```

That directory is gitignored and owned by this tool. The tracked compose file
stays the stack everyone shares, so an experiment never reaches anyone's
`git status`, and what you started persists across sessions.

One file each rather than one shared file, and the reason is worth knowing: a
daemon holds a project as its file was when it loaded it, so a container added
to a loaded project stays invisible until a whole-project `compose::restart` —
which stops everything else in that project. A new file is a new project, so
starting the fifth on-demand worker leaves the other four running.

`x` stops one without deleting its file, so `s` starts it again. Delete the file
(or the directory) to forget it.

Workers that are not Rust binaries show `registry` instead of a state: they
install from the registry rather than from this tree, with
`iii trigger compose::add worker=<name>`.

### Workers from another directory

Press **`a`** in the dashboard. A browser opens beside the repo — where a
sibling checkout like `harness-e2e` lives — and labels every directory by what
it holds:

```
┌ ~/project/iii   2/33 ────────────────────────────────────────┐
│  harness-e2e               1 worker                          │
│  iii.main                                                    │
│  workers                   added                             │
│  workers.feat-needle       4 new of 63                       │
│   _   Enter add or open · → look inside · ⌫ up · Esc         │
└──────────────────────────────────────────────────────────────┘
```

One rule: a directory that offers something new is added; anything else is
opened. So `harness-e2e` is added, `iii.main` is opened, and a worktree of this
repo reads `0 new of 60` and opens — the repo already provides those names, and
the label says so before you press anything. `→` looks inside one anyway.
Typing filters; Backspace clears the filter, then goes up a level.

What you add joins the table as **its own group**, so where a worker came from
stays visible:

```
── stack (13) ──
── repo (60) ──
── harness-e2e (1) ──
```

Select a group header and press `x` to drop that directory. Nothing stops —
a container started from there keeps running, it just leaves the list. The
directories are remembered in `harness/.workers-dev/worker-dirs`, so the next
session opens with them.

The same thing without the dashboard:

```bash
workers-dev --worker-dir ~/project/iii/harness-e2e
# or, once, in your shell profile:
export WORKERS_DEV_WORKER_DIRS=~/project/iii/harness-e2e
```

The directory *is* a worker when it carries an `iii.worker.yaml`; otherwise its
children are scanned, so a path to another monorepo's root works too. The repo
is read first, so a name it already uses is not replaced by a stranger, and a
worker the stack declares never appears in the list at all.

There is no auto-scan of sibling directories, deliberately: a checkout with ten
worktrees beside it would offer seventy copies of every worker.

A worker that ships its own `worker-compose.yaml` is taken at its word — its
container declaration is used as written, with only `worker:` repointed at the
real directory, `start_after` dropped (the containers it names are not in this
project) and the env file appended. `harness-e2e` refuses to start without the
`config_override` its own file carries, and this is how it gets it.

## The dashboard

The table has two groups: `stack`, what the tracked compose file declares, and
`repo`, everything else this repo ships. Each row carries its compose state
(`ready`, `failed`, `stopped`, or `—` for something never started), pid,
UI-watch flag and last error. Row 0 is pinned:
`compose (daemon)`, whose log is the daemon's own output — the startup tree, the
adoption lines, the managed engine's pid and every `error[CODE]`.

The footer always lists the keys the selected row accepts, and only those: the
daemon row offers `^u` and nothing else, a repo worker that installs from the
registry offers nothing at all, and `w` appears only on a worker that ships a
watchable `ui/`. `Enter` opens the same list with a line of explanation each —
the keys keep working while it is open, so it teaches the keyboard rather than
replacing it.

An action never takes the keyboard: it runs detached and reports in the footer,
because a cold container can take minutes and the daemon owns its lifecycle
either way. `Esc` cancels only the project start — the one operation compose
registers; a restart this dashboard asks for passes an id compose never
registers, so there is nothing to cancel and `Esc` says nothing rather than
lying.

While an operation is running, the state column comes from compose's
`compose-operation` feed (`queued` → `starting` → `ready`/`failed`) with a
counter in the header. This matters on a cold checkout: `compose::status` cannot
see inside its own operation, so without the feed thirteen containers would read
`stopped` for the several minutes cargo spends compiling them.

| Key | Action |
| --- | --- |
| `↑`/`↓`, `k`/`j`, `g`/`G` | select |
| `s` | start: `compose::up` a container, or add a repo worker on demand |
| `x` | stop: `compose::down` — for a stack container, confirms with its dependents |
| `r` | `compose::restart` — this container only, no graph |
| `Ctrl+u` | `compose::up` the whole project |
| `w` | toggle the injectable-UI watcher for this container |
| `d` | `start_after` and dependents, with live state |
| `f`, `PgUp`/`PgDn` | follow / scroll the log pane |
| `+`/`-` | move the divider |
| `/` | filter by name |
| `a` | browse for another directory of workers (remembered) |
| `x` on a group header | stop offering that directory |
| `Esc` | cancel the running operation (`compose::cancel`) |
| `Enter` | what this row can do — the keys that apply to it, and what they mean here |
| `?` | keys |
| `q` | quit: `l` leave running · `s` stop everything · `Esc` cancel |
| mouse | click a row to select it; the wheel scrolls whichever pane is under the pointer |

Only button and SGR reporting are enabled (`?1000h` + `?1006h`), not crossterm's
`EnableMouseCapture` — that also turns on any-motion reporting, which wakes the
event loop for every cell the pointer crosses and, under tmux, takes drag-select,
double-click-copy and middle-click paste away from the pane. Selecting text to
copy keeps working.

`s` is per container on purpose: a project-wide `compose::up` rolls back
everything it started if one container fails, so a bad thirteenth worker would
undo twelve good ones. `Ctrl+u` is there when you want exactly that.

## Injectable-UI watchers

For a container whose `ui/` declares a `watch` script, `w` starts `pnpm run
watch` there and sets `III_<WORKER>_UI_WATCH=1`, arming the `iii-console-ui`
poller so open console tabs hot-swap the asset on every rebuild (see
`docs/sops/injectable-console-ui.md`). `--ui-watch` turns on every available one.

The flag reaches the container through the same env file as the API keys: the
compose file declares
`env_file: [${WORKERS_DEV_ENV_FILE:-workers-dev.env.example}]`, `workers-dev`
points that variable at the gitignored `harness/.env.workers-dev` and writes the
flags there. compose re-reads an `env_file` at every container spawn, which is
why a toggle is one `compose::restart` and not a whole-stack bounce.

Two consequences worth knowing:

- **Keep `harness/workers-dev.env.example`.** compose validates every declared
  `env_file` when it first loads the project, so deleting it breaks every
  compose call against this project, `compose::status` included.
- `w` only works against a daemon `workers-dev` started. One you attached to
  resolved `${WORKERS_DEV_ENV_FILE}` at its own launch, and that is out of
  reach; the key says so rather than bouncing a container for nothing.

Watchers are `workers-dev`'s own children and stop when it does, on `q` and on
SIGTERM/SIGHUP alike.

## Two worktrees at once

A compose daemon is one per namespace per engine, and the managed engine's lock
is per namespace machine-wide — so a second worktree needs both its own
namespace and its own port:

```bash
III_ENGINE_PORT=49135 workers-dev --namespace wt2
```

The branch badge in the header (`⎇ feat/my-branch`) and the terminal title say
which instance you are looking at.

## Configuration

There is none. `workers-dev.yaml` is gone — workers, dependencies, environment
and the engine URL live in `harness/worker-compose.yaml`, and the two things
that are genuinely per-checkout are the flag and the env var above. A leftover
`workers-dev.yaml` is a startup error rather than a silently ignored file.

## Troubleshooting

**The table says `stopped` for everything and nothing moves.** Select the
`compose (daemon)` row. On a first run each container is a cold `cargo build`;
the daemon log shows which one is compiling.

**`no compose daemon in this namespace`.** The engine is up but nothing serves
`compose::*` there. Quit and relaunch — that spawns one — or check that
`--namespace` matches the compose file's `namespace:`.

**A managed start fails on the engine port.** An engine from a previous unclean
shutdown still holds it. `pgrep -af 'iii:[ce]:'` finds it; note that
`workers-dev` attaches to a reachable engine rather than fighting it.
