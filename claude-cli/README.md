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
in the terminal, on first use. `ANTHROPIC_API_KEY` in the terminal host's
environment skips that.

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

The simplest flow is the terminal itself: open the page and run `/login`
(or `claude auth login`, `--console` for API billing instead of the
subscription). It authenticates the HOST, so a headless
[`claude-code`](https://github.com/iii-hq/workers/tree/main/claude-code) worker
on the same host and the same `CLAUDE_CONFIG_DIR` picks up the same
credentials — one login covers both surfaces.

For a host with no one at the keyboard, run `claude setup-token` once (it needs
a browser once, and returns a long-lived subscription token) and put the result
in the environment the `shell` worker starts with; or set `ANTHROPIC_API_KEY`
there for metered billing. Either way the badge tells you which one won.

## Functions

| Function | Purpose |
|---|---|
| `claude-cli::terminal::describe` | What a session runs: program, argv, cwd, env. The page passes it straight to `shell::pty::open`. Internal. |
| `claude-cli::activity` | One Claude Code hook event in, AgentEvent frames out. Internal, and `trace_hidden` — the signal is the stream, not the delivery. |
| `claude-cli::auth::status` | Which plan a session spends (see Billing). Agent-denied. |
| `claude-cli::ui-content` | Console page assets. Internal. |

Nothing here is an agent tool: a terminal is opened by a person, from the
console.
