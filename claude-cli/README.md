# claude-cli

Claude Code as a terminal on the iii console. The worker installs the CLI on
the terminal host, equips a workspace with the iii skills and engine notes,
and opens Claude in a `shell::pty` session on its own console page — always
Claude, never a shell. Every prompt and tool call the session runs is streamed
onto `agent::events`, so the console shows this terminal's turns exactly like
a headless agent worker's.

Its siblings: [`claude-code`](https://github.com/iii-hq/workers/tree/main/claude-code)
runs Claude Code headless over the bus (no terminal), and
[`pi-cli`](https://github.com/iii-hq/workers/tree/main/pi-cli) is this worker
for the pi agent.

## Install

```bash
iii worker add claude-cli
```

`iii worker add` fetches the worker and its dependencies, and the engine starts
it the next time it boots.

### Companion workers

| Worker | Why |
|---|---|
| [`shell`](https://github.com/iii-hq/workers/tree/main/shell) ≥ 0.12 | Owns the terminal. `shell::pty::open` runs the CLI, `shell::exec` installs it and equips the workspace, `coder::read-file` reads the files back. The workspace must be inside shell's jail (`fs.host_roots`). |
| [`console`](https://github.com/iii-hq/workers/tree/main/console) | Renders the injected **claude** page. |
| `configuration` (built in) | Holds the settings below. |

Anthropic credentials are not part of the worker: Claude asks how to log in,
in the terminal, on first use, and they belong to the machine the `shell`
worker runs on (see [Logging in](#logging-in) — that is not necessarily this
worker's machine). `ANTHROPIC_API_KEY` in that environment skips the prompt.

## Quickstart

1. Open the console. A **claude** page appears in the nav; opening it starts a
   session. First run installs the CLI and the skills, so it takes a minute.
2. Answer the login prompt in the terminal.
3. Ask Claude to build something on the engine. The workspace notes and the
   installed iii skills teach it how to register functions and triggers.
4. Watch the turn arrive on the console's agent views, or read the stream
   directly:

```bash
iii trigger stream::list stream_name=agent::events
iii trigger claude-cli::terminal::describe   # what a session runs, and where
```

The page keeps a per-tab lease, so a reload or a pane move reattaches to the
same live Claude rather than starting a second one. `shell::pty::sessions`
lists what is actually running.

## Configuration

Settings live in the `configuration` worker under the **claude-cli** entry —
edit them on the console's Workers tab; they hot-reload, and a change re-runs
the workspace preparation for the next session.

```yaml
executable: ""            # path to `claude` on the terminal host; empty = resolve on PATH
args: []                  # extra argv for every session
workspace_dir: ""         # empty = `claude-cli` under shell's primary root
auto_install: true        # install Claude Code from https://claude.ai/install.sh when missing
setup_workspace: true     # keep skills, engine notes, and the activity hooks in place
events_stream: agent::events   # where turns land (read once at boot)
```

## What the workspace gets

On every boot, when `setup_workspace` is on:

- The `iii-hq/iii` skills, once per workspace (`npx skills add iii-hq/iii
  --all`), with a minimal `package.json` first — the skills CLI installs at
  the nearest manifest, and without one they land above the workspace where
  Claude does not look.
- `CLAUDE.md`, inside an `<!-- iii:begin -->` block the worker rewrites.
  Anything outside the markers is the operator's and survives.
- `.claude/settings.json` hooks for `SessionStart`, `SessionEnd`,
  `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, and `Stop`. Each posts its
  payload to `claude-cli::activity` with the `iii` CLI — the bus, so it works
  whether or not the terminal host is this worker's host. Only these keys are
  rewritten; the rest of the file is left alone.

## Billing: which plan a session spends

The status bar under the terminal says it, and `claude-cli::auth::status`
answers it on the bus:

| Badge | Means |
|---|---|
| `team subscription · you@example.com · org` | A Claude subscription login on the terminal host pays. |
| `API key billing · ANTHROPIC_API_KEY` | An API key pays, per token. |
| `not signed in` | No credentials yet — run `/login` in the terminal. |

The rule has a trap in it, which is why the badge exists: **an API key beats a
subscription login.** With `ANTHROPIC_API_KEY` set, `claude auth status` still
reports `authMethod: claude.ai` (the account IS signed in) while the CLI bills
the key — it says so itself ("ANTHROPIC_API_KEY or another auth source is set
and takes precedence over your claude.ai login") and a bogus key 401s the turn.
So the badge reads `apiKeySource`, not `authMethod`, and the tooltip spells out
that the login is signed in but not billed.

The key is read from the environment of the `shell` worker that runs the
session, so that is where to set or unset it.

`claude-cli::auth::status` stays agent-denied in `iii-permissions.yaml`: it
carries the operator's account and organization. The console page reaches it as
a user-initiated call, which is not the agent path.

## Logging in

The simplest flow is the terminal itself: open the page and run `/login` (or
`claude auth login`; `--console` for API billing instead of the subscription).
The credentials land in the home directory of whatever runs the session — the
`shell` worker's — and every later session of this terminal reuses them.

**Who else sees that login depends on how the other worker is deployed, not on
which machine it is.** Compose starts a container one of two ways
(`iii-compose/src/lifecycle.rs`):

| Container | Start | Home directory |
|---|---|---|
| `path://…` (local checkout), or a registry worker whose payload is a **binary** (Rust `deploy: binary`, e.g. `shell`) | host process | the host's |
| a registry worker whose payload is a **bundle** (Node `deploy: bundle`, e.g. `claude-code`, and this worker) | microVM, own rootfs | the guest's |

Only the container's own config directory is shared into a VM (virtiofs), and
compose v1 has no volume field, so a VM-booted worker cannot read the host's
`~/.claude`. Consequences:

- This terminal is unaffected either way: the CLI runs inside the `shell`
  worker, which is a host process, so the login is the host's and this worker's
  own rootfs never matters.
- A headless [`claude-code`](https://github.com/iii-hq/workers/tree/main/claude-code)
  run from a local checkout is a host process too, and shares the login.
- A headless `claude-code` installed from the registry is in a microVM and does
  **not** share it. Give it credentials through the channels a VM has:
  `environment` / `env_file` on its container (`ANTHROPIC_API_KEY`, or a
  long-lived subscription token from `claude setup-token`), or the
  `auth-credentials` vault over the bus — which is exactly why that vault
  exists, and why `provider-claude-code` reads it instead of the file.

### If the worker that owns the terminal is virtualized

Everything above holds because `shell` is a binary payload and therefore a host
process today. If binary workers get VM-booted too, the terminal keeps working
— the CLI, the workspace, and the login simply move into that guest, and this
worker still reaches them over the bus — but three things change, and they are
worth knowing before that day:

- The login belongs to that guest's rootfs. Nothing on the host shares it, and
  it survives restarts only as long as the guest's state does.
- The workspace is inside the guest, so the agent no longer sees the operator's
  checkout unless the guest is given it.
- The activity hooks call the `iii` CLI, which may not exist in that guest. The
  worker probes for it and reports the answer as `activity_bridge` on
  `claude-cli::terminal::describe` (empty = the hooks are installed but mute,
  and the `detail` field says so). That is the first thing to check if a
  terminal works while `agent::events` stays empty.

For a terminal host with no one at the keyboard, the same two options apply:
`claude setup-token` once (needs a browser once, returns a long-lived
subscription token) or `ANTHROPIC_API_KEY`, in the environment the `shell`
worker starts with. Either way the badge says which one won.

## Functions

| Function | Purpose |
|---|---|
| `claude-cli::terminal::describe` | What a session runs: program, argv, cwd, env — the page passes it straight to `shell::pty::open` — plus `activity_bridge` and `detail`, which say whether the hooks can reach the bus. Internal. |
| `claude-cli::activity` | One Claude Code hook event in, AgentEvent frames out. Internal, and `trace_hidden` — the signal is the stream, not the delivery. |
| `claude-cli::auth::status` | Which plan a session spends (see Billing). Agent-denied. |
| `claude-cli::ui-content` | Console page assets. Internal. |

Nothing here is an agent tool: a terminal is opened by a person, from the
console.
