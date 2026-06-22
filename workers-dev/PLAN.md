# workers-dev — local dev orchestrator

A single TUI/CLI to spin up, supervise, restart (rebuild), and tail the seven
core workers that make up the harness stack, against a locally-running iii
engine.

Managed workers:

```
session-manager   llm-router   context-manager   approval-gate
provider-anthropic   provider-openai   harness
```

---

## 1. Mental model

- The **iii engine** is the bus. It runs separately (`iii --use-default-config`,
  bridge WS on `:49134`). harness-dev does **not** reimplement it.
- Each worker is a **child process** = `cargo run --manifest-path <worker>/Cargo.toml -- --url ws://127.0.0.1:49134`.
  - "Restart" = kill the child, then `cargo run` again → cargo rebuilds if the
    source changed, so restart *is* rebuild-and-run.
- harness-dev owns three responsibilities per worker:
  1. **Process lifecycle** — spawn / kill / restart, track PID + run state.
  2. **Log capture** — pipe child stdout+stderr into an in-memory ring buffer
     (last N lines) *and* tee to a file under `harness-dev/.logs/<worker>.log`.
  3. **Connection status** — poll the engine's `engine::workers::list` and match
     by worker name to learn `connected` / `available` / `disconnected`.
- A worker has two independent state axes that the dashboard shows together:
  - **process state**: `stopped | building | running | crashed`
    (from our own child handle + exit codes)
  - **engine state**: `connected | available | disconnected | unknown`
    (from `engine::workers::list`)

---

## 2. Tech stack

Standalone Rust crate (matches the repo; gives us a real TUI):

- `ratatui` + `crossterm` — TUI dashboard.
- `clap` (derive) — subcommands.
- `tokio` — async process supervision, log pumps, status poll loop.
- `tokio::process::Command` — spawn `cargo run`, capture piped stdout/stderr.
- `serde` / `serde_json` — parse `engine::workers::list`.
- `serde_yaml` — read each worker's `iii.worker.yaml` to build the dep graph.

Engine status is read by **shelling out to `iii trigger engine::workers::list
--json '{}'`** and parsing stdout JSON. (Rationale: zero coupling to a specific
`iii-sdk` version, and it's a dev tool. If we later want push updates instead of
polling, swap to an `iii-sdk` WS client — isolated behind one `EngineClient`
trait.)

---

## 3. Crate layout

```
harness-dev/
├── Cargo.toml
├── PLAN.md
├── workers.toml              # the worker registry (see §4) — or derive from yaml
├── src/
│   ├── main.rs               # clap entrypoint, dispatch
│   ├── registry.rs           # load workers + build dependency / dependents graph
│   ├── supervisor.rs         # spawn/kill/restart, child handles, exit watching
│   ├── logs.rs               # per-worker ring buffer + file tee + tail reader
│   ├── engine.rs             # EngineClient: poll engine::workers::list -> status
│   ├── graph.rs              # topo sort + reverse-dependency (dependents) walk
│   ├── state.rs              # shared AppState (Arc<Mutex<…>>) the TUI renders
│   └── tui/
│       ├── mod.rs            # event loop, key handling
│       ├── dashboard.rs      # the multi-worker grid view
│       └── logview.rs        # full-screen single-worker log pager
└── .logs/                    # gitignored; captured stdout/stderr per worker
```

`harness-dev` has **no `iii.worker.yaml`**, so the repo's CI worker-discovery
ignores it.

---

## 4. Worker registry & dependency graph

Source of truth for deps is each worker's existing `iii.worker.yaml`. Two options:

- **A (recommended): derive at runtime.** On startup, read the 7
  `iii.worker.yaml` files, take their `dependencies` keys, and keep only edges
  pointing at other *managed* workers (drop engine built-ins like
  `configuration`, `iii-state`, `iii-queue`, `iii-directory`). No duplication —
  the graph always matches the yaml.
- **B: a small `workers.toml`** listing dir + bin + extra run args, for cases
  where we want to override (e.g. add `--config ./dev.yaml`, set env). Start with
  A; add B only if per-worker overrides are needed.

Computed graph:

```
tier 0:  session-manager      llm-router
tier 1:  approval-gate        context-manager   provider-anthropic   provider-openai
tier 2:  harness
```

- **Start order**: tier 0 → 1 → 2 (a topological sort).
- **Dependents map** (reverse edges), used by restart:

  | restart this | also restart, in order |
  |---|---|
  | session-manager | approval-gate, harness |
  | llm-router | context-manager, provider-anthropic, provider-openai, harness |
  | context-manager | harness |
  | approval-gate | harness |
  | provider-anthropic | harness |
  | provider-openai | harness |
  | harness | — |

---

## 5. Supervisor design

Per worker we hold:

```rust
struct Worker {
    name: String,
    dir: PathBuf,                 // <repo>/<name>
    deps: Vec<String>,            // managed-only
    child: Option<Child>,         // tokio process handle
    proc_state: ProcState,        // Stopped|Building|Running|Crashed
    started_at: Option<Instant>,
    restarts: u32,
    logs: RingBuffer<Line>,       // last 500 lines, color-tagged stdout/stderr
}
```

- **spawn(name)**: `cargo run --manifest-path <dir>/Cargo.toml -- --url ws://127.0.0.1:49134`,
  `Stdio::piped()` on stdout+stderr. Set `proc_state = Building`; flip to
  `Running` on the first captured line (or after the SDK "registered" log line —
  we can pattern-match `register_worker` output). Two tokio tasks per child pump
  stdout/stderr → `logs` ring buffer + the `.logs/<name>.log` file.
- **exit watching**: `child.wait()` in a task; on exit, set `Crashed` (nonzero)
  or `Stopped` (we asked it to stop) and surface the exit code in the dashboard.
- **kill(name)**: send SIGTERM to the child, escalate to SIGKILL after a grace
  period. (Important: `cargo run` spawns the worker as a *grandchild*. Spawn each
  child in its **own process group** and signal the group, or the worker binary
  outlives cargo. On Unix: `Command::process_group(0)` then `kill(-pgid)`.)

---

## 6. Engine status polling

- One background task every ~1s: run `iii trigger engine::workers::list --json '{}'`,
  parse `{ workers: [...] }`, build `name -> status` map, write into shared state.
- Match engine entries to managed workers by `name`. Unmatched managed worker →
  `disconnected`. Engine unreachable (trigger errors/timeouts) → all `unknown`
  and a banner "engine down — run `iii --use-default-config`".
- Optional `--manage-engine` flag: harness-dev also spawns/supervises the engine
  itself as worker tier "-1". Default off (assume engine already running).

---

## 7. Log handling

- In-memory **ring buffer** (e.g. 500 lines) per worker, each line tagged
  `stdout`/`stderr` + timestamp; dashboard shows the **last 5**.
- Simultaneously **tee to `harness-dev/.logs/<name>.log`** (truncated on each
  fresh `up`, appended across restarts within a session).
- `harness-dev logs <name> [-f]` reads from the ring buffer / file; `-f` follows.

---

## 8. CLI surface

```
harness-dev                      # launch the TUI dashboard (default)
harness-dev up                   # start all 7 in dependency order, then dashboard
harness-dev up <name>            # start one (+ its deps first if not running)
harness-dev down                 # stop all (reverse dependency order)
harness-dev restart <name>       # rebuild+restart <name>, then its dependents (§4)
harness-dev restart --all        # rolling restart, dependency order
harness-dev logs <name> [-f]     # tail a worker's logs (follow with -f)
harness-dev status               # one-shot table (proc + engine state), no TUI
```

`restart <name>` algorithm:
1. compute `[name] + dependents(name)` ordered topologically,
2. for each: kill → `cargo run` (rebuild) → wait until engine reports
   `connected` (or timeout),
3. report per-step result.

---

## 9. TUI dashboard layout

Default view — a grid/list of all workers, each cell showing status badges +
last 5 log lines:

```
┌ harness-dev ───────────────────────────────  engine: ● connected (:49134) ┐
│                                                                            │
│ ┌ session-manager ───────── ● running  ◆ connected   pid 8121  ↻0 ──────┐ │
│ │ 12:03:01  INFO registered worker session-manager v1.2.0               │ │
│ │ 12:03:01  INFO 14 functions, 6 trigger types registered               │ │
│ │ 12:03:02  INFO listening for session::* invocations                   │ │
│ │ 12:03:09  INFO session::append ok (turn 3)                            │ │
│ │ 12:03:10  INFO snapshot flushed                                       │ │
│ └───────────────────────────────────────────────────────────────────────┘ │
│ ┌ llm-router ────────────── ● running  ◆ connected   pid 8122  ↻1 ──────┐ │
│ │ 12:03:01  INFO router up; 0 providers bound                           │ │
│ │ …                                                                     │ │
│ └───────────────────────────────────────────────────────────────────────┘ │
│ ┌ provider-anthropic ────── ● building ◇ disconnected …rebuilding…  ↻2 ─┐ │
│ │ 12:04:55  Compiling provider-anthropic v0.4.1                         │ │
│ └───────────────────────────────────────────────────────────────────────┘ │
│ ┌ harness ───────────────── ◼ crashed  ◇ disconnected exit=101  ↻0 ─────┐ │
│ │ 12:03:40  ERROR llm-router unreachable: connection refused            │ │
│ └───────────────────────────────────────────────────────────────────────┘ │
│  (… context-manager, approval-gate, provider-openai …)                     │
│                                                                            │
├────────────────────────────────────────────────────────────────────────────┤
│ ↑/↓ select   r restart(+deps)   s stop   u start   l logs   q quit         │
└────────────────────────────────────────────────────────────────────────────┘
```

Badges:
- proc: `● running` `◼ crashed` `○ stopped` `● building`
- engine: `◆ connected` `◈ available` `◇ disconnected` `? unknown`
- `↻N` = restart count this session; `pid`, `exit=` shown when relevant.

Keys: `↑/↓` select a worker, `r` restart selected (+dependents), `s` stop,
`u` start, `l` open full-screen log pager for the selected worker (the
"console scroll" pager — scrollable, `/` search, `f` follow), `q` quit
(prompts to stop children or leave them running).

---

## 10. Build phases

1. **Skeleton + registry**: crate, clap, read the 7 yaml files, build + print
   the dep graph and topo order (`harness-dev status` stub). Validates §4.
2. **Supervisor**: spawn/kill one worker via `cargo run`, process-group signal
   handling, exit watching. `up <name>` / `down`.
3. **Log capture**: ring buffer + file tee + `logs <name> [-f]`.
4. **Engine polling**: `EngineClient` + `status` table merging proc+engine state.
5. **TUI dashboard**: render the grid, last-5 logs, badges, live refresh.
6. **Restart-with-dependents** + the log pager + key bindings.
7. Polish: `--manage-engine`, readiness-wait on restart, config overrides.

---

## 11. Open decisions (defaults chosen, easy to flip)

- **debug vs release builds**: default `cargo run` (debug, fast rebuilds);
  `--release` flag to opt in.
- **engine lifecycle**: default assume it's already up; `--manage-engine` to let
  harness-dev own it.
- **registry source**: derive from `iii.worker.yaml` (option A); add
  `workers.toml` only when per-worker run overrides are needed.
