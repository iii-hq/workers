# iii: a Claude Code plugin for the iii engine

Agent profiles and skills from a running iii engine's `iii-directory` worker,
as slash commands and MCP tools. It connects with `iii-sdk`, so it works in two
places:

- **Inside the `claude-code` worker**, with nothing to install. The worker
  carries this folder in its bundle and installs it wherever Claude Code runs:
  the OS temp dir for headless `claude::run` turns (when `iii_context` is on),
  and `<workspace>/.iii-plugin` on the terminal host for terminal sessions
  (when `setup_workspace` is on). The worker adds what only it can: hooks that
  report every turn onto `claude::on-agent-event`, the `iii-runtime` skill,
  and the `/iii:<profile>` commands, generated fresh for each session.
- **In a standalone `claude`** on your own machine, with no worker involved:

  ```sh
  claude --plugin-dir /path/to/claude-code/iii-plugin
  ```

Requires Node 22+. The server is a single bundled file (`server/index.mjs`)
with no runtime dependencies.

## Commands

Claude Code namespaces plugin commands as `/<plugin>:<command>`, so they all
start with `/iii:` and typing `/iii:` completes them.

| Command | What it does |
| --- | --- |
| `/iii:profile [<profile> [task]]` | Act as any agent profile, hidden ones included. No arguments lists them. |
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

## Tools

The `iii` MCP server gives the agent `iii_status`, `iii_list_agents`,
`iii_get_agent`, `iii_list_skills` and `iii_get_skill`, and every skill as an
`iii-skill://<id>` resource. Everything is read live; the connection opens on
first use, so `claude` starts cleanly before the engine does.

## Finding the engine

First match wins:

1. `--url` on the server command line
2. `III_PLUGIN_URL` (this plugin only)
3. the plugin's `engine_url` option (Claude Code asks for it when you enable
   the plugin; also read from `CLAUDE_PLUGIN_OPTION_ENGINE_URL`)
4. `III_URL`, then `III_ENGINE_URL`
5. **auto-detect**: TCP-probe ports `49134`, `49135`, `49136` on `127.0.0.1`
   and `localhost`; the first one that answers is used

Addresses can be `ws://host:port`, `wss://...`, `host:port`, `:port` or a bare
port. An explicit address is never replaced by probing: if it is down, you get
that error, not a different engine. Change the probe set with
`III_PLUGIN_PORTS=49000,49134` and `III_PLUGIN_HOSTS=...`.

Ask `iii_status` (or run `node server/index.mjs status`) to see which engine
was chosen and why.

## Server commands

```sh
node iii-plugin/server/index.mjs sync     # regenerate /iii:<profile> commands now
node iii-plugin/server/index.mjs status   # which engine, and does it answer
node iii-plugin/server/index.mjs export   # profiles as native ~/.claude/agents/iii-<id>.md
node iii-plugin/server/index.mjs export --skills --target ./.claude --include-hidden
```

`export` is a snapshot for people who want the profiles in `/agents` without
the plugin; re-run it to refresh. Profile models are not copied (iii profiles
name models across providers).

## Building and the committed server

`server/index.mjs` is a build output, and it is **committed**: a plugin
installed from git (a Claude Code marketplace) is a copy of this folder with
no build step of its own. The worker does not depend on the committed copy:
CI's deploy build (`build:bundle`) rebuilds the server from source and embeds
this folder in the worker bundle.

After changing anything under `src/iii-plugin/`, the lockfile, or the build
script, rebuild and commit both files:

```sh
pnpm run build:iii-plugin   # writes server/index.mjs and server/BUILD.json
```

`server/BUILD.json` records a hash of every input the bundle was built from
and of the output. CI runs `npm run check:committed-build`, which fails when
an input changed without a rebuild, or when `server/index.mjs` was edited by
hand. Generated `/iii:<profile>` commands are git-ignored.
