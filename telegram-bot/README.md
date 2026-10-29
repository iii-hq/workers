# telegram-bot

A Telegram bridge to the harness agent stack. The worker owns Telegram UX — commands, inline keyboards, live message edits, and approval prompts — and delegates turns, streaming, and durability to `harness`, `session-manager`, and `approval-gate`.

Update ingress uses a session-manager-style `updates` adapter: **polling** (default, no public URL) or **webhook** (HTTPS production).

## Install

```bash
iii call compose::add worker=harness worker=session-manager worker=llm-router \
  worker=context-manager worker=approval-gate worker=telegram-bot
```

## Quickstart (polling — default)

1. Set configuration (configuration id `telegram-bot`):

```yaml
bot_token: "${TELEGRAM_BOT_TOKEN}"
verbosity: minimal
default_model:
  provider: anthropic
  id: claude-sonnet-4
functions_allow:
  - "shell::*"
```

2. Message the bot on Telegram. The worker long-polls `getUpdates` in the background; user text is sent to `harness::send` with update-id idempotency; assistant output streams back via `session::on-message-updated` edits.

Commands: `/start` (new session + model picker or default model), `/stop` (cancel turn), `/model` (re-pick model), `/help`, `/thinking` (reasoning depth), `/verbosity` (transcript detail), `/settings`.

Each `/start` clears the chat's active harness session. The next message after model selection creates a fresh session (harness-assigned ID); later messages continue that session until the next `/start`.

Assistant output streams via `sendMessageDraft` when available (Bot API 9.3+), with `editMessageText` fallback. Model thinking blocks stream via `sendRichMessageDraft` (`RichBlockThinking`). Drafts are finalized to persistent messages on turn completion.

The worker does not send a `sendChatAction(typing)` indicator: Telegram has no stop-typing API and each action lingers ~5s on clients, leaving the indicator visible for seconds after the answer arrives. Progress is shown by streamed thinking/answer drafts (draft transport) or the streamed message bubble (edit transport) instead.

When a tool call needs approval, the bot posts an inline keyboard: **Approve**, **Reject**, and **Approve always** (per-session grant via `approval::approve-always`).

## Webhook mode (production)

```yaml
bot_token: "${TELEGRAM_BOT_TOKEN}"
updates:
  name: webhook
  config:
    base_url: "https://your-engine.example"   # iii engine root only
    secret: "your-webhook-secret"             # recommended
```

`base_url` is the **public root of your iii engine** — the bot appends its own
path (`/telegram-bot/webhook`) to build the URL handed to Telegram, so operators
never repeat the path. Selecting the `webhook` adapter (at boot or via
hot-reload) does everything automatically, with no restart:

1. observes `http::on-request` with the `POST /telegram-bot/webhook` route
   (waiting up to 10 s for the http worker to accept it), then
2. calls Telegram `setWebhook` with `{base_url}/telegram-bot/webhook`.

Switching back to `polling` reverses both — it calls `deleteWebhook` and then
disposes the route's observer. No manual step is required.

> The legacy full-URL form (`url: https://…/telegram-bot/webhook`) is still
> accepted for backward compatibility. A `secret` is strongly recommended:
> without it, anyone who learns the URL can inject forged updates.

`POST /telegram-bot/set-webhook` remains available to manually re-arm Telegram
(e.g. if it dropped the webhook) without changing configuration.

## Configuration

All fields hot-reload through the `configuration` worker — no restart required, including `bot_token` and `updates` adapter swaps.

| Field | Description |
|---|---|
| `bot_token` | **Required.** Telegram Bot API token (`${TELEGRAM_BOT_TOKEN}`) |
| `updates` | Ingress adapter: `polling` (default) or `webhook` — see below |
| `default_model` | Skip model picker when set (`provider` + `id`) |
| `verbosity` | `none` \| `minimal` \| `high` \| `debug` — controls transcript mirroring |
| `default_thinking_level` | Optional harness reasoning depth: `minimal` \| `low` \| `medium` \| `high` \| `xhigh` |
| `streaming` | Draft streaming — `transport` (`auto`/`draft`/`edit`), `draft_id_seed`, `draft_throttle_ms`, `create_settle_ms` |
| `steering_mode` | `steering` (default, harness merge) or `fifo` (local queue) |
| `functions_allow` | Globs for `harness::send` `options.functions.allow` |
| `system_prompt` | Optional system prompt on every send |
| `system_prompt_mode` | `override` (default, replaces the harness prompt) or `enrich` (appends `system_prompt` to it) |
| `channel_context` | `auto` (default, injects the built-in Telegram channel-context prompt) or `off` |
| `timeout_ms` | Timeout for harness, approval, state, and configuration RPCs (default `10000`) |

### Telegram channel awareness

By default (`channel_context: auto`) the bot layers a built-in **channel-context
prompt** onto every send, telling the agent that it is talking to a Telegram
user: its reply text is the only thing the user sees (no visible console — so
"log a reminder" reaches no one), how to format for Telegram, and how to reach
the user *later* (reminders/schedules) by targeting this session. The prompt
carries the live `chat_id` and `session_id`.

The agent reaches the user out-of-band via **`telegram-bot::notify`**
(`{ session_id, text, parse_mode? }`) — a session-scoped send that delivers only
to the chat bound to that session. For reminders the agent observes a time
source (`harness::on-timer` or `cron::on-tick`, through `harness::observe`) with
`telegram-bot::notify` (fixed text) or `harness::send` (generated reply)
targeting this session.

Composition with `system_prompt_mode`: `enrich` layers `[channel context] +
[system_prompt]` on top of the harness's built-in prompt; `override` uses
`[channel context] + [system_prompt]` alone. Set `channel_context: off` to drop
the channel layer entirely.

### `updates` adapter

**Polling** (default):

```yaml
updates:
  name: polling
  config:
    timeout_seconds: 50   # optional, Telegram max 50
```

**Webhook**:

```yaml
updates:
  name: webhook
  config:
    base_url: "https://your-engine.example"   # iii engine root; required
    secret: "your-webhook-secret"             # recommended
```

The bot derives the Telegram webhook URL as `{base_url}/telegram-bot/webhook`
and observes `http::on-request` for the route, or disposes that observer, as
the adapter is switched to/from
`webhook` — see [Webhook mode](#webhook-mode-production).

Verbosity levels:

- `none` — final assistant text, errors, approvals only (thinking still shown via native rich draft)
- `minimal` — same as `none` for transcript mirroring; thinking uses RichBlockThinking regardless
- `high` — also mirrors function call blocks
- `debug` — also mirrors function result entries

Persisted at `./data/configuration/telegram-bot.yaml` when using the default fs adapter.

Final assistant messages are rendered as Telegram HTML (LLM markdown converted to bold, code, links, lists). Live streaming edits stay plain text until finalization to avoid broken partial markup.

## Trace correlation

Telegram ingress and harness bindings share OpenTelemetry baggage so the console can group traces by session and turn:

- **`iii.session.id`** — harness session id (or `pending-{chat_id}` before the first send in a chat)
- **`iii.message.id`** — `tg-{update_id}` at ingress; harness `turn_id` after `harness::send` returns and for binding handlers

`harness::send` receives `options.metadata` with `{ session_id, message_id, surface: "telegram" }` for engine passthrough. Polling mode stamps baggage per update in the poller; webhook mode stamps it in the HTTP handler.

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./tests/fixtures/config.yaml
cargo test
UPDATE_GOLDENS=1 cargo test   # after schema changes
```

HTTP endpoints (http worker default `http://127.0.0.1:3000`), each an
`http::on-request` observer:

- `POST /telegram-bot/webhook` — Telegram update ingress. Observed **only while the `webhook` adapter is active**; disposed in polling mode.
- `POST /telegram-bot/set-webhook` — manually (re-)register the Telegram webhook from config. Always available.

## Observers

The worker provides no observable event functions of its own. It observes
other workers' functions; each observer stays `pending` until that worker
registers the function.

| Observed function | Args / filters | Target | Target receives |
|---|---|---|---|
| `http::on-request` | args `{ method: "POST", path: "/telegram-bot/webhook" }` (webhook adapter only) | `telegram-bot::webhook` | the HTTP request; returns `{ ok }` |
| `http::on-request` | args `{ method: "POST", path: "/telegram-bot/set-webhook" }` | `telegram-bot::set-webhook` | the HTTP request |
| `session::on-message-added` | filters `{ "input.message.role": { "$in": ["assistant", "function_result"] } }` | `telegram-bot::on-message-added` | `MessageAddedEvent` (`deliver: output`) |
| `session::on-message-updated` | filters `{ "input.message.role": { "$in": ["assistant"] } }` | `telegram-bot::on-message-updated` | `MessageUpdatedEvent` (`deliver: output`) |
| `session::on-status-changed` | none | `telegram-bot::on-status-changed` | `StatusChangedEvent` (`deliver: output`) |
| `harness::on-turn-completed` | none | `telegram-bot::on-turn-completed` | `TurnCompletedEvent` (`deliver: output`) |
| `approval::on-pending-created` | none | `telegram-bot::on-pending-created` | `PendingApprovalRecord` (`deliver: output`) |
| `approval::on-pending-resolved` | none | `telegram-bot::on-pending-resolved` | `PendingResolvedEvent` (`deliver: output`) |
| `engine::on-workers-changed` | filters `{ "input.event": { "$in": ["worker_connected", "worker_metadata_updated", "worker_disconnected"] } }` | `telegram-bot::__on-approval-gate-worker` | the event; re-reads `engine::workers::list` to track approval-gate presence |
| `configuration::on-updated` | filters `{ "input.id": { "$eq": "<config id>" }, "input.event_type": { "$eq": "configuration:updated" } }` | `telegram-bot::on-config-change` | the configuration event (`deliver: output`) |

The approval observers exist from boot, so an approval-gate that connects
later is observed with no extra step. In code, a session observer is:

```rust
iii.observe(
    Source::function("session::on-message-updated")
        .filters(json!({ "input.message.role": { "$in": ["assistant"] } }))
        .deliver(Deliver::Output),
    "telegram-bot::on-message-updated",
);
```
