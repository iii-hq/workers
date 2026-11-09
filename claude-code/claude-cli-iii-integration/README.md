# claude-cli-iii-integration: a Claude Code plugin for iii

Agent profiles and skills from a running iii engine's `iii-directory` worker,
served to Claude Code over MCP. It connects with `iii-sdk`, so it works in two
places:

- **Inside the `claude-code` worker**, with nothing to install. The worker
  carries the plugin in its own bundle and writes it out wherever Claude Code
  runs: the OS temp dir for headless `claude::run` turns (when `iii_context`
  is on), and `<workspace>/.claude-cli-iii-integration` on the terminal host
  for terminal sessions (when `setup_workspace` is on). Both get
  `--plugin-dir` for it, and find the engine through the `III_URL` the worker
  exports. A `.build-id` hash means an unchanged plugin is not rewritten.
- **In a standalone `claude`** on your own machine, with no worker involved:

  ```sh
  npm run build:iii-integration          # once, from claude-code/
  claude --plugin-dir /path/to/claude-code/claude-cli-iii-integration
  ```

Requires Node 22+. The server is a single bundled file
(`server/index.mjs`) with no runtime dependencies.

## What Claude Code gets

| Surface | What |
| --- | --- |
| Tools | `iii_status`, `iii_list_agents`, `iii_get_agent`, `iii_list_skills`, `iii_get_skill` |
| Prompts | one slash command per visible agent profile (listed under the plugin's `iii` MCP server in `/`); it adopts that profile, with an optional task |
| Resources | `iii-skill://<id>` for every skill (`@`-mention them) |

Everything is read live: there is no copy to go stale. The connection opens on
first use, so `claude` starts cleanly before the engine does and attaches when
it appears.

## Finding the engine

First match wins:

1. `--url` on the server command line
2. `III_INTEGRATION_URL` (this plugin only)
3. the plugin's `engine_url` option (Claude Code asks for it when you enable
   the plugin; also read from `CLAUDE_PLUGIN_OPTION_ENGINE_URL`)
4. `III_URL`, then `III_ENGINE_URL`
5. **auto-detect**: TCP-probe ports `49134`, `49135`, `49136` on `127.0.0.1`
   and `localhost`; the first one that answers is used

Addresses can be `ws://host:port`, `wss://...`, `host:port`, `:port` or a bare
port. An explicit address is never replaced by probing: if it is down, you get
that error, not a different engine. Change the probe set with
`III_INTEGRATION_PORTS=49000,49134` and `III_INTEGRATION_HOSTS=...`.

Ask `iii_status` (or run `node server/index.mjs status`) to see which engine
was chosen and why.

## Native subagents and skills (`sync`)

If you want the profiles in `/agents` and the skills auto-invoked without
going through MCP, write them as native Claude Code files:

```sh
node claude-cli-iii-integration/server/index.mjs sync                 # ~/.claude/agents/iii-<id>.md
node claude-cli-iii-integration/server/index.mjs sync --skills        # plus ~/.claude/skills/<id>/SKILL.md
node claude-cli-iii-integration/server/index.mjs sync --target ./.claude --include-hidden
```

This is a snapshot: run it again to refresh. Profile models are not copied
(iii profiles name models across providers); a profile's preloaded skills
become an instruction to read them with `iii_get_skill`.
