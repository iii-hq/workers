# harness

Harness meta-worker. Composes the modular workers that back the iii chat
surface.

The harness boots first; reads its `iii.worker.yaml` so the engine knows
which sibling workers to spawn; registers `harness::status`,
`bridge::trigger`, `bridge::info`, `ui::subscribe`, `ui::unsubscribe`,
and `harness::fs::read_inline`; and wires the upstream fanout pumps
(agent events, sessions, approvals, cost, workers, skills, prompts).
The agent's starting context is driven by `turn-orchestrator`'s
`system_default_skills` config (fetched per chat); anything else is
loaded on demand via `directory::skills::get`.

- [`harness`](iii://harness/index)
  - [`harness::status`](iii://harness/index) — returns the bundle
    name, version, and the list of expected runtime workers.
  - [`bridge::trigger`](iii://harness/index) — forwards
    `{function_id, payload}` from the browser onto the iii bus.
  - [`ui::subscribe`](iii://harness/index) / [`ui::unsubscribe`](iii://harness/index) —
    per-browser interest in a session (or all sessions); pumps push
    `ui::*` triggers back.

## Workers

Runtime workers the harness assumes are on the iii bus. Source of truth:
`harness/iii.worker.yaml`. See [iii-directory](iii://iii-directory/index)
for download/list semantics.

| Worker | Role |
|---|---|
| `iii-state` | Engine: keyed state store (`state::get`, `state::list`, …). |
| `iii-queue` | Engine: durable work queues for `iii.trigger(... action: Enqueue)`. |
| `iii-stream` | Engine: append-only event streams (`stream::set`, `stream::list`, …). |
| `iii-bridge` | Engine: HTTP/WS bridge for browser callers. |
| `iii-http` | Engine: HTTP trigger type for `bridge::trigger`. |
| `turn-orchestrator` | Drives a single chat turn: tool dispatch, provider routing, approvals. |
| `provider-router` | Routes provider calls through `llm-budget` + `session::inbox`. |
| `session` | Per-session conversation tree, messages, workspace state, and the usage/cost ledger consumed by `llm-budget`. |
| `models-catalog` | Available models, pricing, capability flags. |
| `hook-fanout` | Multiplexes upstream `agent::events` to per-session subscribers. |
| `policy-denylist` | Static deny rules for tool-call gating. |
| `shell` | Sandboxed shell/filesystem (`shell::exec`, `shell::fs::*`). |
| `provider-anthropic` | Anthropic chat completions backend. |
| `provider-openai` | OpenAI chat completions backend. |
| `auth-credentials` | Per-provider credentials storage (`auth::*`). |
| `llm-budget` | Budget enforcement and `budget::*` introspection. |
| `iii-directory` | Filesystem-backed skill/prompt registry (`skills::*`, `prompts::*`, `skill::fetch`, `iii://`). |
| `approval-gate` | Approval workflow for tool calls (`approval::*`). |
| `iii-sandbox` | Engine: in-process sandboxed code execution. |
