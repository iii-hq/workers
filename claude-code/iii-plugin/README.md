# iii: a Claude Code plugin for the iii engine

Agent profiles, skills and the engine itself, as slash commands and MCP
tools. The plugin has no server of its own: its `iii` MCP server is the
engine's [`mcp-server`](../../mcp-server/README.md) worker, over HTTP. Every
call therefore goes through `mcp-server`, and so through rbac-proxy's rules.
It works in two places:

- **Inside the `claude-code` worker**, with nothing to install. The worker
  carries this folder in its bundle and installs it wherever Claude Code runs:
  the OS temp dir for headless `claude::run` turns (when `iii_context` is on),
  and `<workspace>/.iii-plugin` on the terminal host for terminal sessions
  (when `setup_workspace` is on). It points the plugin, and its
  `default-mcp` SessionStart hook, at the engine's mcp-server (the http
  worker's address plus `/mcp`, or `III_PLUGIN_MCP_URL`) and adds what only
  it can: hooks that report every turn onto
  `claude::on-agent-event`, the `iii-runtime` skill, and the `/iii:<profile>`
  commands, generated fresh for each session.
- **In a standalone `claude`** on your own machine:

  ```sh
  claude --plugin-dir /path/to/claude-code/iii-plugin
  ```

Both need `mcp-server` running on the engine, behind rbac-proxy: see its
README. Without it the `iii` tools are missing, and every `/iii:*` command
says so instead of guessing. The SessionStart hooks need Node 22+.

## Connecting to mcp-server

The plugin's `mcp_url` option (Claude Code asks for it when you enable the
plugin) is where mcp-server answers: the http worker's address plus its
`api_path`. The default is `http://127.0.0.1:3111/mcp`. The plugin's CLI
(below) reads `--url`, then `III_PLUGIN_MCP_URL`, then that option.

Ask the agent for `iii_status`: it reports the server, its `access_control`
(`rbac-proxy` and the rules in force) and the session's `active_profile`.

## Commands

Claude Code namespaces plugin commands as `/<plugin>:<command>`, so they all
start with `/iii:` and typing `/iii:` completes them.

| Command | What it does |
| --- | --- |
| `/iii:profile [<profile> [task]]` | Act as any agent profile, hidden ones included. No arguments shows the active profile, then lists them all. |
| `/iii:<profile> [task]` | One per visible profile, e.g. `/iii:writer draft the notes`. |
| `/iii:skills [<skill> [task]]` | Load any skill. No arguments lists them; a non-id word searches. |

Every command is `disable-model-invocation: true`: it is yours to type, and it
adds nothing to a session's context until you do (measured with `/context`).
A profile named `profile`, `skills` or `skill` gets `/iii:profile-<name>`
instead. (A plugin command cannot be named `skill`: Claude Code then loads
none of the plugin's commands.)

Standalone, the `/iii:<profile>` commands come from `sync` (below), which the
plugin's SessionStart hook runs in the background: a profile added to the
engine appears from the next session on.

## The active profile

Every session starts on `default-mcp`: the plugin's SessionStart hook
(`cli/index.mjs session-start`) reads it from mcp-server and adds it to the
session's context. (mcp-server's instructions ask the agent to load it too,
but Claude Code defers MCP tools until they are needed, so a session that
starts on a small question would not.) Loading a profile with `/iii:profile
<id>` or `/iii:<profile>` activates it with `iii_get_agent { activate: true }`.

mcp-server records the active profile per MCP session, and `iii_status`
reports it as `active_profile` (`default-mcp` until another is activated), so
`/iii:profile` shows it even after the conversation was compacted. A renewed
MCP session (mcp-server ends one idle for 30 minutes; each `claude -p` run
is a new one) starts on `default-mcp` again; `/iii:profile` says so when the
conversation follows another profile.

## Tools

The tools are mcp-server's:

| Tool | What it does |
| --- | --- |
| `iii_call` | Call any function by id with a JSON payload. |
| `iii_observe` | Observe a function; each matching invocation becomes an observation. |
| `iii_observations_poll` | Read and remove this session's buffered observations. |
| `iii_observations_sse` | Stream observations on the session's GET stream instead. |
| `iii_observers`, `iii_unobserve` | This session's observers; remove one. |
| `iii_secret_store` | Store a credential in the vault without putting it in a payload. |
| `iii_status` | The server, its access control, and the session's `active_profile`. |
| `iii_list_agents`, `iii_get_agent`, `iii_list_skills`, `iii_get_skill` | Profiles and skills. |

See mcp-server's README for each one in full.

## iii mode

Claude Code has no plugin-defined permission modes, so the iii mode is the
plugin's `iii` agent plus a hook, on top of whichever built-in mode is active:

- **The `iii:iii` agent** (`agents/iii.md`) makes the whole session work
  through the engine: load `default-mcp`, find a function, read its contract,
  `iii_call` it, `iii_observe` instead of polling. The worker starts every
  session with `--agent iii:iii` (an operator-configured `--agent` wins);
  standalone, run `claude --plugin-dir ... --agent iii:iii`.
- **Read-only tools are pre-approved** by a PreToolUse hook: status,
  profiles, skills, `iii_observations_poll`, `iii_observations_sse`,
  `iii_observers`, `iii_unobserve`. `iii_call`, `iii_observe` and
  `iii_secret_store` are never pre-approved: they stay with the session's
  permission mode (and, for a headless turn, the worker's approval gate).

Every `/iii:*` command carries the same rule: `iii_call` instead of the `iii`
CLI, `iii_observe` instead of polling, and `agent_call` / `harness::observe`
in harness-written profiles mean `iii_call` / `iii_observe` here.

## CLI

```sh
node iii-plugin/cli/index.mjs status   # which mcp-server, and what it reports
node iii-plugin/cli/index.mjs sync     # regenerate /iii:<profile> commands now
node iii-plugin/cli/index.mjs export   # profiles as native ~/.claude/agents/iii-<id>.md
node iii-plugin/cli/index.mjs export --skills --target ./.claude --include-hidden
node iii-plugin/cli/index.mjs session-start  # what the SessionStart hook adds
```

Each takes `--url <mcp-url>`. `export` is a snapshot for people who want the
profiles in `/agents` without the plugin; re-run it to refresh. Profile models
are not copied (iii profiles name models across providers).

## Building and the committed CLI

`cli/index.mjs` is a build output, and it is **committed**: a plugin installed
from git (a Claude Code marketplace) is a copy of this folder with no build
step of its own. It has no dependencies (plain `fetch` to mcp-server), so it
is a few kilobytes. CI's deploy build (`build:bundle`) rebuilds it from source
and embeds this folder in the worker bundle.

After changing anything under `src/iii-plugin/`, the lockfile, or the build
script, rebuild and commit both files:

```sh
pnpm run build:iii-plugin   # writes cli/index.mjs and cli/BUILD.json
```

`cli/BUILD.json` records a hash of every input the bundle was built from and
of the output. CI runs `npm run check:committed-build`, which fails when an
input changed without a rebuild, or when `cli/index.mjs` was edited by hand.
Generated `/iii:<profile>` commands are git-ignored.
