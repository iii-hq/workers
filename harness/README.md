<div align="center">

# harness

**A thin, durable turn loop that turns a model plus a few iii workers into an agent.**

<p>
  <a href="#install"><img alt="Install: iii worker add harness" src="https://img.shields.io/badge/install-iii%20worker%20add%20harness-0a84ff?style=flat-square"></a>
  <a href="../LICENSE"><img alt="License: Apache 2.0" src="https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square"></a>
  <a href="https://www.rust-lang.org"><img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-rust-orange?style=flat-square&logo=rust&logoColor=white"></a>
</p>

<p><sub><b>INSTALLS WITH HARNESS</b></sub></p>

<p>
  <a href="../iii-directory"><img alt="iii-directory" src="https://img.shields.io/badge/iii--directory-2b2b2b?style=flat-square"></a>
  <a href="../session-manager"><img alt="session-manager" src="https://img.shields.io/badge/session--manager-2b2b2b?style=flat-square"></a>
  <a href="../context-manager"><img alt="context-manager" src="https://img.shields.io/badge/context--manager-2b2b2b?style=flat-square"></a>
  <a href="../provider-anthropic"><img alt="provider-anthropic" src="https://img.shields.io/badge/provider--anthropic-2b2b2b?style=flat-square"></a>
  <a href="../provider-openai"><img alt="provider-openai" src="https://img.shields.io/badge/provider--openai-2b2b2b?style=flat-square"></a>
  <a href="../shell"><img alt="shell" src="https://img.shields.io/badge/shell-2b2b2b?style=flat-square"></a>
  <a href="../web"><img alt="web" src="https://img.shields.io/badge/web-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="iii-state" src="https://img.shields.io/badge/iii--state-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="iii-queue" src="https://img.shields.io/badge/iii--queue-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="iii-cron" src="https://img.shields.io/badge/iii--cron-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="iii-stream" src="https://img.shields.io/badge/iii--stream-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="iii-observability" src="https://img.shields.io/badge/iii--observability-2b2b2b?style=flat-square"></a>
  <a href="https://github.com/iii-hq/iii"><img alt="configuration" src="https://img.shields.io/badge/configuration-2b2b2b?style=flat-square"></a>
</p>

</div>

`harness` is the thin, durable turn loop that turns a model plus a few iii
workers into an agent. It takes an incoming message, persists it, assembles a
context, streams a completion, runs any function calls the model requests, and
repeats until the turn stops — all as durable, resumable steps so a crash or
restart picks up mid-turn. It wires [`session-manager`](../session-manager)
(transcript), [`context-manager`](../context-manager) (token budgeting, soft
dependency), and [`llm-router`](../llm-router) (generation); install those
alongside it for the full loop.

## Install

```bash
iii worker add harness
```

`iii worker add` fetches the binary, registers the `harness` configuration
entry, and the engine starts the worker on the next `iii start`. Add the
workers it wires so the loop has something to call:

```bash
iii worker add session-manager
iii worker add llm-router
iii worker add context-manager
```

The harness enqueues turn steps on the engine's built-in `default` queue, provided
by the `iii-queue` worker (see [`engine.config.yaml`](engine.config.yaml)).

## Quickstart

Send a message and let the loop run; render the conversation by binding
`session-manager`'s triggers, and observe turn boundaries with
`harness::turn-completed`.

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // Fire-and-return: persists the user message and kicks off a turn.
    let accepted = iii
        .trigger(TriggerRequest {
            function_id: "harness::send".into(),
            payload: json!({
                "message": "Summarise the repo README",
                "model": "claude-sonnet-4",
                "provider": "anthropic",
                "options": { "functions": { "allow": ["shell::*", "fs::*"] } }
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await?;
    println!("{accepted:#?}"); // { session_id, turn_id, accepted: true }
    Ok(())
}
```

Want a typed result back? Add an output contract
(`options.output: { type: "json", schema }`) and read the result off the
[`harness::turn-completed`](#custom-trigger-types) event — bind it filtered to
your session and the result arrives when the turn finishes.

The agent-facing function surface is deny-by-default: with no `functions.allow`
globs, every model-requested call is refused and the harness is a plain chat
loop. Allow functions in per-send (`options.functions.allow`) and gate them
with the optional [`approval-gate`](../approval-gate) sibling.

The full function reference (every `harness::*` id and its request/response
schema) lives in the code and `iii worker info harness`.

Building a consumer — a chat UI, a Telegram/WhatsApp bridge, a cron worker, or
any event-driven loop on top of the harness? Start with the integration
contract in
[`architecture/integration.md`](architecture/integration.md): the functions to
trigger, the triggers to bind, and the canonical consumer patterns.

## Configuration

The `harness` configuration entry is owned by the `configuration` worker; every
field hot-reloads (no restart). The fields a deployment is most likely to tune:

```yaml
default_max_turns: 16            # per-turn generate-step cap when a send omits it
default_pending_timeout_ms: 1800000  # parked pending-call (sub-agent / hold) wait guard
max_depth: 3                     # sub-agent depth budget
max_children: 5                  # sub-agent fan-out budget
sweep_expression: "0 * * * * *"  # cron for the pending-call expiry sweep
```

Other keys (RPC timeouts, stream coalescing, idempotency TTL, validation
retries) and their defaults live in [`src/config.rs`](src/config.rs).

## System prompt

When `options.system_prompt` is omitted (or empty), the harness assembles the
engine-grounded identity prompt at send time: four provider-specific variants
(`anthropic`, `openai` → gpt, `kimi`, and a step-by-step default for local
runtimes) selected from `provider`, plus an optional `mode` (`plan` | `ask` |
`agent`) that prepends a short operating-mode paragraph. A non-empty
`system_prompt` is combined with the built-in prompt per
`options.system_prompt_strategy`: `override` (default) uses it verbatim, while
`enrich` appends it to the built-in prompt. Prompt bodies live in
[`prompts/`](prompts/) and are tested in [`src/prompt/tests.rs`](src/prompt/tests.rs).

## Custom trigger types

The harness emits two async orchestration trigger types siblings and consumers
bind to, and registers five synchronous hook points operator-trusted siblings
plug into in-path. Bind with the standard two-step pattern.

| Trigger type | Kind | Fires / runs |
|---|---|---|
| `harness::turn-started` | async event | A turn began executing (first loop step). |
| `harness::turn-completed` | async event | A turn reached a terminal status (`completed` / `cancelled` / `failed`), carrying the result. |
| `harness::hook::pre-turn` | sync hook | First step of a turn, before any model spend. May veto. |
| `harness::hook::pre-generate` | sync hook | After context assembly, before generation. May extend the system prompt, append messages, or veto. |
| `harness::hook::post-generate` | sync hook | After the final assistant message. Observe only. |
| `harness::hook::pre-trigger` | sync hook | After the allow/deny policy passes, before the target runs. May deny, hold, or rewrite arguments. |
| `harness::hook::post-trigger` | sync hook | After the target returns, before the result is persisted. May rewrite the result. |

Event configs accept `{ session_id?, parent_session_id? }`; hook configs accept
`{ functions?, priority?, timeout_ms?, on_error? }`. See the spec at
[`tech-specs/2026-06-agentic/harness.md`](../tech-specs/2026-06-agentic/harness.md)
for the hook contract and chain semantics.
