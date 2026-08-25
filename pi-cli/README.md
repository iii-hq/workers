# pi-cli

The [pi](https://pi.dev) coding agent as a terminal on the iii console. The
worker installs the CLI on the terminal host, equips a workspace with the iii
skills and engine notes, and opens pi in a `shell::pty` session on its own
console page — always pi, never a shell. A pi extension the worker installs
reports every prompt and tool call onto `agent::events`, so the console shows
this terminal's turns exactly like a headless agent worker's.

Its siblings: [`pi`](https://github.com/iii-hq/workers/tree/main/pi) runs the
same agent headless over the bus (no terminal), and
[`claude-cli`](https://github.com/iii-hq/workers/tree/main/claude-cli) is this
worker for Claude Code.

## Install

```bash
iii worker add pi-cli
```

`iii worker add` fetches the worker and its dependencies, and the engine starts
it the next time it boots.

### Companion workers

| Worker | Why |
|---|---|
| [`shell`](https://github.com/iii-hq/workers/tree/main/shell) ≥ 0.12 | Owns the terminal. `shell::pty::open` runs the CLI, `shell::exec` installs it and equips the workspace, `coder::read-file` reads the files back. The workspace must be inside shell's jail (`fs.host_roots`). |
| [`console`](https://github.com/iii-hq/workers/tree/main/console) | Renders the injected **pi** page. |
| `configuration` (built in) | Holds the settings below. |

Model credentials are not part of the worker: pi asks in the terminal
(`/login`), or reads a provider key from the terminal host's environment.

## Quickstart

1. Open the console. A **pi** page appears in the nav; opening it starts a
   session. First run installs the CLI and the skills, so it takes a minute.
2. Log in with `/login` if the host has no provider key.
3. Ask pi to build something on the engine. The workspace notes and the
   installed iii skills teach it how to register functions and triggers.
4. Watch the run arrive on the console's agent views, or read the stream
   directly:

```bash
iii trigger stream::list stream_name=agent::events
iii trigger pi-cli::terminal::describe   # what a session runs, and where
```

The page keeps a per-tab lease, so a reload or a pane move reattaches to the
same live pi rather than starting a second one. `shell::pty::sessions` lists
what is actually running.

The status bar carries a font-size stepper (8–40 px, 14 by default; Ctrl or ⌘
+ scroll does the same). The size is one value for every terminal in the
console — shell's panes included — so it is set once, not per page. The
terminal itself stays dark whatever the console theme is: an agent CLI paints
its own interface in colors chosen for a dark terminal and never learns the
page went light.

## Configuration

Settings live in the `configuration` worker under the **pi-cli** entry — edit
them on the console's Workers tab; they hot-reload, and a change re-runs the
workspace preparation for the next session.

```yaml
executable: ""            # path to `pi` on the terminal host; empty = resolve on PATH
args: ["-a", "--use-theme", "dark"]   # `-a` trusts the workspace; the theme matches the page
workspace_dir: ""         # empty = `pi-cli` under shell's primary root
auto_install: true        # install pi from https://pi.dev/install.sh when missing
setup_workspace: true     # keep skills, engine notes, and the activity extension in place
events_stream: agent::events   # where turns land (read once at boot)
auth_provider: anthropic  # whose credentials the billing badge reports
```

`-a` is in the default args for a reason: pi loads project-local extensions
only in a trusted directory, and asks about trust otherwise. Without it, every
session opens with a prompt and never loads the extension that reports what pi
did. Drop the flag if you would rather answer that question yourself.

## What the workspace gets

On every boot, when `setup_workspace` is on:

- The `iii-hq/iii` skills, once per workspace (`npx skills add iii-hq/iii
  --all`), with a minimal `package.json` first — the skills CLI installs at
  the nearest manifest. pi reads `.agents/skills` from the working directory,
  which is where they land.
- `AGENTS.md`, inside an `<!-- iii:begin -->` block the worker rewrites.
  Anything outside the markers is the operator's and survives.
- `.pi/extensions/iii-activity.ts`, which posts pi's lifecycle events
  (`session_start`, `before_agent_start`, `tool_execution_start`/`end`,
  `agent_end`, `session_shutdown`) to `pi-cli::activity` with the `iii` CLI —
  the bus, so it works whether or not the terminal host is this worker's host.
  It is discovered from the workspace, so a session an operator starts by hand
  in the same directory reports its turns too.

## Billing: which plan a session spends

The status bar under the terminal says it, and `pi-cli::auth::status` answers
it on the bus. pi keeps credentials per provider, so the badge reports the
provider named by `auth_provider`:

| Badge | Means |
|---|---|
| `anthropic subscription` | An OAuth (subscription) login pays. |
| `anthropic API key billing` | An API key pays, per token. |
| `anthropic: not signed in` | No credentials for that provider yet — run `/login`. |
| `anthropic ready` | pi has credentials but did not report their kind. |

It runs `pi auth check --provider <p> --json` on the terminal host and never
passes `--credentials`: the page needs the KIND of credential, never the
credential. `pi auth check` exits 1 for a provider that is not ready and still
prints the JSON that says why, so the exit code is not read as the answer.

`pi-cli::auth::status` stays agent-denied in `iii-permissions.yaml`. The
console page reaches it as a user-initiated call, which is not the agent path.

## Logging in

The simplest flow is the terminal itself: open the page and run `/login`. The
credentials land in the home directory of whatever runs the session — the
`shell` worker's — and every later session of this terminal reuses them.

**Who else sees that login depends on how the other worker is deployed.**
Compose runs a `path://` container and a registry **binary** payload as host
processes, and a registry **bundle** payload (Node `deploy: bundle`, which is
`pi` and this worker) in a microVM with its own rootfs; only the container's
config directory is shared in, and compose v1 has no volume field. So a
headless [`pi`](https://github.com/iii-hq/workers/tree/main/pi) from a local
checkout shares the login, while one installed from the registry does not —
give that one its provider key through `environment` / `env_file` on its
container. This terminal is unaffected either way: the CLI runs inside the
`shell` worker, a host process.

### If the worker that owns the terminal is virtualized

Everything above holds because `shell` is a binary payload and therefore a host
process today. If binary workers get VM-booted too, the terminal keeps working
— pi, the workspace, and the login move into that guest — but the login belongs
to that guest's rootfs, the workspace is no longer the operator's checkout, and
the activity extension calls the `iii` CLI, which may not exist there. The
worker probes for it and reports the answer as `activity_bridge` on
`pi-cli::terminal::describe` (empty = the extension is installed but mute, with
`detail` saying so) — the first thing to check if a terminal works while
`agent::events` stays empty.

For a terminal host with no one at the keyboard, put the provider's key in the
environment the `shell` worker starts with. Either way the badge says which one
won.

## Functions

| Function | Purpose |
|---|---|
| `pi-cli::terminal::describe` | What a session runs: program, argv, cwd, env — the page passes it straight to `shell::pty::open` — plus `activity_bridge` and `detail`, which say whether the extension can reach the bus. Internal. |
| `pi-cli::activity` | One pi extension event in, AgentEvent frames out. Internal, and `trace_hidden` — the signal is the stream, not the delivery. |
| `pi-cli::auth::status` | Which plan a session spends (see Billing). Agent-denied. |
| `pi-cli::ui-content` | Console page assets. Internal. |

Nothing here is an agent tool: a terminal is opened by a person, from the
console.
