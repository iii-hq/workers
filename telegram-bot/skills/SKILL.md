---
name: telegram-bot
description: >-
  Bridge a harness agent onto Telegram — turn inbound chat messages into
  `harness::send` turns and stream assistant output, model pickers, and inline
  tool-call approvals back into the chat.
---

# telegram-bot

The telegram-bot is a bridge between a Telegram bot and the harness agent
stack. It owns the Telegram UX — slash commands, model-picker keyboards, live
message edits, and approval prompts — and delegates turn execution, streaming,
and durability to `harness`, `session-manager`, and `approval-gate`. Inbound
text becomes a `harness::send` turn; the assistant's output flows back as
Telegram messages. It is not a general bot framework and exposes no scriptable
"send a message" surface of its own.

Reach for it to put an existing harness agent in front of Telegram users.
Prerequisites: the sibling stack installed (`harness session-manager llm-router
context-manager approval-gate`) and a `telegram-bot` configuration entry with
a `bot_token`. A model is chosen per chat through the `/start` picker or fixed
with `default_model`; the agent only gets the tools you grant via
`functions_allow`. Update ingress is either long-polling (default, no public
URL) or an HTTPS webhook, and every config field hot-reloads.

## When to Use

- Expose a harness agent to Telegram users with streaming replies and live edits.
- Let a chat user pick or switch models and steer or cancel a running turn from chat.
- Approve or reject held tool calls inline from Telegram (Approve / Reject / Approve always).
- Mirror a session's transcript into a chat at a configurable verbosity.

## Boundaries

- No public "send arbitrary text to a chat" function. Output reaches a chat only
  as the rendered result of a harness turn on a session mapped to that chat.
- No scheduler, cron, or timer. The worker never fires delayed or proactive
  messages on its own — see Message flow for how a reminder is actually delivered.
- Renders output only for sessions it holds a `chat_id ↔ session_id` mapping for;
  a turn on an unrelated session produces nothing in Telegram.
- Turn execution, tool-calling, and approval policy live in `harness` and
  `approval-gate`; this worker is only the Telegram surface. Deploying it without
  the sibling stack is unsupported.

## Functions

HTTP-triggered (operator/ingress only — not agent-callable):

- `telegram-bot::webhook` — receive a Telegram update in webhook mode and
  route commands, messages, and callback queries.
- `telegram-bot::set-webhook` — register the configured webhook URL with
  Telegram.

Internal bindings (auto-registered against sibling triggers; not called directly):

- `telegram-bot::on-message-added` — post each new assistant or
  function_result entry into the chat (from `session::message-added`).
- `telegram-bot::on-message-updated` — stream throttled assistant edits (from
  `session::message-updated`).
- `telegram-bot::on-status-changed` — show a typing indicator while the
  session is working (from `session::status-changed`).
- `telegram-bot::on-turn-completed` — finalize streaming, post the outcome
  toast, and drain the FIFO queue (from `harness::turn-completed`).
- `telegram-bot::on-pending-created` — show an inline approval keyboard for a
  held call (from `approval::pending-created`).
- `telegram-bot::on-pending-resolved` — clear the approval prompt once the
  call resolves (from `approval::pending-resolved`).
- `telegram-bot::on-config-change` — hot-reload configuration.

## Message flow

The worker registers no trigger type for others to bind. It is a bidirectional
bridge between one Telegram chat and one harness session, and that round-trip is
the whole job.

Inbound: a Telegram update arrives by long-poll or webhook and lands in the
`telegram-bot::webhook` handler. Slash commands are handled locally; any other
text is sent to `harness::send` with the chat's session, the selected model, the
configured `system_prompt`, and the `functions_allow` policy. The first message
in a chat creates the session and persists a `chat_id ↔ session_id` mapping;
later messages continue that session until the next `/start`.

Outbound: as the turn runs, `session-manager`, `harness`, and `approval-gate`
emit events. The worker's bound handlers consume them and render into the
originating chat — resolving which chat by looking the session up in the
`chat_id ↔ session_id` mapping. Assistant text streams via message edits (or
native drafts) and is finalized on `harness::turn-completed`.

Reminder round-trip: a user asking the bot to "remind me at 5pm" gets an
immediate reply in the same turn — that is just the normal inbound-to-outbound
path. The later notification is a *second* turn: when the time comes, something
must call `harness::send` on that same chat-bound `session_id`, and its assistant
output then streams back through the same bindings. Because this worker has no
scheduler and no push API, the timing lives elsewhere (an external scheduler,
cron, or another worker that holds the `session_id`); the bridge's only role is
routing that session's output back to the chat.
