# Search evaluation catalog

`worker-compose.search-eval.yaml` is an isolated, local catalog source. It
does not replace `worker-compose.yaml` and excludes only workers without a
static searchable surface, those that require a remote peer or own a listener
at boot, and workers with unsafe boot-time work.

## Prerequisites

- `iii` and the Rust toolchain
- Node.js 22 and `pnpm`
- `uv`

Capture after the catalog is stable:

```sh
iii compose --up --file worker-compose.search-eval.yaml
python3 scripts/search_eval_catalog.py --port 49134 --accept
```

The collector targets the `search-eval` namespace by default. The engine port
is required so a capture cannot silently use a different local engine. Use
`--namespace <name>` to capture a different compose namespace.

The collector writes the normal list, internal list, every `functions::info`
batch (never more than 32 ids), errors, and its normalized catalog under
`../target/search-eval/<UTC timestamp>/`. `--accept` is the only mode that updates
`../iii-directory/tests/fixtures/discover_catalog.json`.

## Manifest ledger

| Worker | Decision | Reason |
| --- | --- | --- |
| a2ui | included | Local Rust worker. |
| acp | excluded | Stdio-only; no static interface. |
| approval-gate | included | Local state-backed worker. |
| bridge | excluded | Remote engine bridge. |
| browser | included | Registers public functions before idle sweeping; Chromium starts only on invocation. |
| canvas | included | Local state-backed worker. |
| claude-code | included | Missing CLI is non-fatal; public functions register before invocation-time validation. |
| code-runner | included | Registers its local in-process runtime surface. |
| codex | included | Configuration failure falls back to defaults; the CLI is invocation-only. |
| compose-ui | included | Independent bus worker with public logs and project functions. |
| computer | excluded | Conditional remote/sandbox session restore before catalog publish. |
| console | excluded | Opens an HTTP listener. |
| context-manager | included | Local catalog and context surface. |
| cron | included | Local scheduler surface. |
| cursor | included | Boot constructs inert factories and registers public surfaces; validation is invocation-only. |
| database | included | Registers without a database connection. |
| devin | included | Empty credentials disable only cloud API calls; boot still registers all functions. |
| document | included | Local document conversion surface. |
| editor | included | Local shell/state-backed surface. |
| email | included | Registers without mail credentials; delivery remains configured at call time. |
| eval | included | Local evaluation surface. |
| fp | included | Local transform surface. |
| github | included | Registers without a token; authenticated operations remain unavailable until configured. |
| grok | included | Configuration failures are non-fatal; functions register without launching Grok. |
| harness | excluded | Defines queues and starts the turn loop at boot. |
| hermes | included | Registers bus functions and an HTTP trigger binding; gateway use is invocation-only. |
| http | excluded | Opens an HTTP listener. |
| iii-directory | included | Local directory and search surface. |
| image-resize | included | Local image conversion surface. |
| llm-router | included | Registers without a provider token. |
| lsp | excluded | Stdio-only; no static interface. |
| mcp | included | Public handler registers; optional HTTP binding failure is non-fatal. |
| memory-consolidate | excluded | Schedules maintenance work at boot. |
| memory | included | Local state-backed memory surface. |
| opencode | included | Missing CLI is non-fatal; startup registers functions without spawning it. |
| opengantry | included | Local bundle registers public functions and a trigger type without remote work. |
| openwiki | excluded | Restart reaping/state writes and persisted cron re-arming, not Wikipedia. |
| pdf | included | Local PDF conversion surface. |
| pi | included | Registers the in-process surface and waits; credentials are invocation-only. |
| provider-anthropic | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-claude-code | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-command-code | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-deepseek | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-github-copilot | included | Public stream, refresh, and login functions register without credentials. |
| provider-kimi | included | Public stream and refresh functions register before non-blocking declaration/refresh. |
| provider-llamacpp | included | Public functions register; an absent model server only makes background refresh fail. |
| provider-openai | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-openai-codex | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-opencode-go | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-openrouter | included | Public stream and refresh functions register before background declaration/refresh. |
| provider-xai | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| provider-zai | excluded | Credential-tolerant boot, but publishes only internal functions; zero normalized/searchable entries. |
| pubsub | included | Registers without a broker connection. |
| queue | included | Uses its local file-backed adapter. |
| rbac-proxy | excluded | Remote boundary proxy listener. |
| sandbox-code-runner | included | Public surface registers without sandbox availability; no sandbox starts until invocation. |
| scrapling | excluded | Requires its browser-image container. |
| security-scan | excluded | Registers scheduled scan work at boot. |
| session-manager | included | Local durable session surface. |
| shell | included | Local shell/filesystem surface. |
| slack | excluded | Requires Slack credentials. |
| state | included | Local state service. |
| storage | included | Registers without object-store credentials. |
| tailscale | included | Registers configuration, functions, and UI without calling the daemon. |
| telegram-bot | excluded | Requires a Telegram bot token. |
| vscode | included | Public lifecycle functions register; VS Code starts only in its handler. |
| web | included | Registers outbound web functions without boot-time network access. |
| workflow | included | Local state/queue-backed workflow surface. |
| worktree | included | Local shell/state-backed worktree surface. |
