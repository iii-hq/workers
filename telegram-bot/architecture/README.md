# telegram-bot architecture

`telegram-bot` is a binary-deployed Rust worker that bridges a Telegram bot to
the iii harness/agent stack. It owns the **Telegram surface** — commands, inline
keyboards, live message edits, draft streaming, approval prompts — and delegates
the **agent loop** (turns, streaming, durability, approvals) to its siblings:
`harness`, `session-manager`, `approval-gate`, and `llm-router`. Durable
bookkeeping lives in the external `state` worker; all operator configuration
lives in the `configuration` worker and hot-reloads without a restart.

## Document map

| Doc | What it covers |
|---|---|
| [README.md](README.md) (this file) | The system in one paragraph and one diagram; the worker's place in the stack; vocabulary. |
| [telegram-api.md](telegram-api.md) | All communication with the **Telegram Bot API**: ingress (polling loop / webhook), the outbound method surface, draft vs edit streaming, secret validation, throttling, file handling, the HTTP client. |
| [configuration.md](configuration.md) | The **configuration** model: schema, the `configuration`-worker integration, boot sequence, hot-reload, and the `updates` adapter lifecycle (polling ↔ webhook and the dynamic HTTP-trigger registration). |
| [internals.md](internals.md) | Everything else: the reactive bindings to siblings, the render/streaming pipeline, the durable KV schema, the per-chat FSM, preference resolution, telemetry/trace correlation, and the concurrency model. |

## The system in one paragraph

A Telegram update arrives — by long-poll (`getUpdates`) in **polling** mode or
by HTTP POST to the engine route in **webhook** mode — and both paths funnel
into one sink (`webhook::process_update`). Commands (`/start`, `/model`, …) and
inline-keyboard callbacks are handled locally; plain user text is forwarded to
`harness::send`, which assigns a session id and runs the agent turn. As the turn
runs, `session-manager` and `harness` emit events (`message-added`,
`message-updated`, `status-changed`, `turn-completed`) that the engine routes
back to this worker's **bindings**. The render pipeline turns each session entry
into Telegram output — streaming live via ephemeral **drafts**
(`sendMessageDraft`) or in-place **edits** (`editMessageText`), then finalizing
to persistent messages at end of turn. When a tool call needs human approval,
`approval-gate` emits `pending-created`, the bot posts an Approve/Reject/Approve-
always keyboard, and the button press resolves it. Per-chat mappings (chat ↔
session, selected model, preferences) and per-entry render bookkeeping are
persisted in the `state` worker so the bridge survives restarts and out-of-order
event delivery.

## The system in one diagram

```
                          Telegram Bot API (api.telegram.org)
                                   ▲            │
                  outbound methods │            │ updates
       sendMessage / editMessageText           │ (message, callback_query)
       sendMessageDraft / sendRichMessageDraft  │
       answerCallbackQuery / sendChatAction      │
       setWebhook / deleteWebhook / getUpdates   │
                                   │      ┌──────┴───────┐
                                   │      │  POLLING:    │  long-poll getUpdates
                                   │      │  background  │  in a bg task
                                   │      │  loop        │
                                   │      │  WEBHOOK:    │  engine HTTP route
                                   │      │  HTTP POST   │  /telegram-bot/webhook
                                   │      └──────┬───────┘
   ┌───────────────────────────────┴─────────────┴───────────────────────────┐
   │                              telegram-bot worker                          │
   │                                                                           │
   │  ingress ──► webhook::process_update ──► commands / callbacks / user text │
   │                                              │                            │
   │   render pipeline ◄── bindings ◄── engine ◄──┤ harness::send (drive turn) │
   │   (draft / edit, chunk,                       │ harness::stop / status     │
   │    verbosity, ordering)                       │ router::models::list       │
   │        │                                      │ approval::resolve / …      │
   └────────┼──────────────────────────────────────┼───────────────────────────┘
            │                                       │
            │ durable bookkeeping                   │ events (triggers)
            ▼                                       ▼
     ┌─────────────┐     ┌──────────┐  ┌─────────────────┐  ┌──────────────┐  ┌─────────────┐
     │ state (KV)  │     │ harness  │  │ session-manager │  │ approval-gate│  │ llm-router  │
     │ scope=      │     │ send/stop│  │ message-added/  │  │ pending-*    │  │ models::list│
     │ telegram-bot│     │ turn-*   │  │ updated/status  │  │ resolve      │  │             │
     └─────────────┘     └──────────┘  └─────────────────┘  └──────────────┘  └─────────────┘

   configuration worker ──(configuration:updated)──► hot-reload (bot_token, adapter, …)
```

## Vocabulary

- **Chat** — a Telegram conversation, keyed by `chat_id` (i64). The unit of
  per-user state.
- **Session** — a harness/`session-manager` conversation, keyed by a
  harness-assigned `session_id` (string). One active session per chat; `/start`
  clears it. Mapped both ways in KV (`chat:{id}:session` ⇄ `session:{sid}:chat`).
- **Turn** — one agent run inside a session, identified by `turn_id`. Started by
  `harness::send`, ended by `harness::turn-completed`.
- **Entry** — one item in a session transcript (assistant message, function call,
  function result), keyed by `entry_id`. Streams in via `message-added` /
  `message-updated`.
- **Revision** — a monotonically increasing version of an entry. The render
  pipeline drops stale (lower) revisions and reconciles higher ones.
- **Adapter** (`updates`) — how Telegram updates reach the worker: **polling**
  (default, no public URL) or **webhook** (engine HTTP route + `setWebhook`).
- **Draft transport** — streaming via ephemeral `sendMessageDraft` /
  `sendRichMessageDraft` previews (Bot API 9.3+), finalized to real messages.
- **Edit transport** — streaming via a persistent `sendMessage` + repeated
  `editMessageText`; the fallback when drafts are unsupported.
- **order_key** — a per-entry append timestamp (ms) used to post new Telegram
  bubbles in transcript order even when events race.
- **Finalize** — the end-of-turn flush that converts live/draft state into
  persistent messages and stops the typing indicator.
- **Verbosity** — how much of the transcript is mirrored: `none` / `minimal` /
  `high` / `debug`.
- **Steering mode** — `steering` folds mid-turn messages into the running turn
  (harness merge); `fifo` queues them locally and drains one per turn.
- **Binding** — a registered handler bound to a sibling's trigger
  (`session::message-added`, `harness::turn-completed`, `approval::pending-created`, …).
- **HTTP trigger** — an engine route (`api_path` + method) that invokes a worker
  function. The webhook ingress route is one; it is registered/removed as the
  `updates` adapter is switched (see [configuration.md](configuration.md)).
