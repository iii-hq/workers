# telegram-bot

A Telegram bridge to the harness agent stack. The worker owns Telegram UX — commands, inline keyboards, live message edits, and approval prompts — and delegates turns, streaming, and durability to `harness`, `session-manager`, and `approval-gate`.

Update ingress uses a session-manager-style `updates` adapter: **polling** (default, no public URL) or **webhook** (HTTPS production).

## Install

```bash
iii worker add harness session-manager llm-router context-manager approval-gate
iii worker add telegram-bot
```

## Quickstart (polling — default)

1. Set configuration (configuration id `telegram-bot`):

```yaml
bot_token: "${TELEGRAM_BOT_TOKEN:}"
verbosity: minimal
default_model:
  provider: anthropic
  id: claude-sonnet-4
functions_allow:
  - "shell::*"
```

2. Message the bot on Telegram. The worker long-polls `getUpdates` in the background; user text is sent to `harness::send` with update-id idempotency; assistant output streams back via `session::message-updated` edits.

Commands: `/start` (new session + model picker or default model), `/stop` (cancel turn), `/model` (re-pick model), `/help`, `/thinking` (reasoning depth), `/verbosity` (transcript detail), `/settings`.

Each `/start` clears the chat's active harness session. The next message after model selection creates a fresh session (harness-assigned ID); later messages continue that session until the next `/start`.

Assistant output streams via `sendMessageDraft` when available (Bot API 9.3+), with `editMessageText` fallback. Model thinking blocks stream via `sendRichMessageDraft` (`RichBlockThinking`). Drafts are finalized to persistent messages on turn completion.

When draft streaming is enabled (`streaming.transport: auto` or `draft`), the worker skips `sendChatAction(typing)` — thinking/answer drafts already show progress. With edit-only fallback, typing is deferred 400ms and cleared when the final `sendMessage` posts (Telegram has no stop-typing API; each `sendChatAction` lasts ~5s on clients unless a bot message arrives).

When a tool call needs approval, the bot posts an inline keyboard: **Approve**, **Reject**, and **Approve always** (per-session grant via `approval::approve-always`).

## Webhook mode (production)

```yaml
bot_token: "${TELEGRAM_BOT_TOKEN:}"
updates:
  name: webhook
  config:
    url: "https://your-engine.example/telegram-bot/webhook"
    secret: "optional-secret"
```

Register (or re-register) with Telegram:

```bash
curl -X POST http://127.0.0.1:3000/telegram-bot/set-webhook
```

Hot-reload to webhook mode also calls `setWebhook` automatically when `url` is set.

## Configuration

All fields hot-reload through the `configuration` worker — no restart required, including `bot_token` and `updates` adapter swaps.

| Field | Description |
|---|---|
| `bot_token` | **Required.** Telegram Bot API token (`${TELEGRAM_BOT_TOKEN:}`) |
| `updates` | Ingress adapter: `polling` (default) or `webhook` — see below |
| `default_model` | Skip model picker when set (`provider` + `id`) |
| `verbosity` | `none` \| `minimal` \| `high` \| `debug` — controls transcript mirroring |
| `default_thinking_level` | Optional harness reasoning depth: `minimal` \| `low` \| `medium` \| `high` \| `xhigh` |
| `streaming` | Draft streaming — `transport` (`auto`/`draft`/`edit`), `draft_id_seed`, `draft_throttle_ms`, `create_settle_ms` |
| `steering_mode` | `steering` (default, harness merge) or `fifo` (local queue) |
| `functions_allow` | Globs for `harness::send` `options.functions.allow` |
| `system_prompt` | Optional system prompt on every send |
| `timeout_ms` | Timeout for harness, approval, state, and configuration RPCs (default `10000`) |

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
    url: "https://your-engine.example/telegram-bot/webhook"  # required
    secret: "optional-secret"
```

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

HTTP endpoints (engine default `http://127.0.0.1:3000`):

- `POST /telegram-bot/webhook` — Telegram update ingress (webhook mode only)
- `POST /telegram-bot/set-webhook` — calls Telegram `setWebhook` (webhook mode only)
