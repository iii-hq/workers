# Communication with the Telegram Bot API

This document covers every interaction between `telegram-bot` and Telegram: how
updates come **in** (the two ingress adapters), how output goes **out** (the
method surface and the streaming transports), and the cross-cutting concerns —
the HTTP client, secret validation, throttling, formatting, and media.

All of it lives in [`src/clients/telegram.rs`](../src/clients/telegram.rs)
(the API client), [`src/ingress.rs`](../src/ingress.rs) (the poller + adapter
supervisor), [`src/functions/webhook.rs`](../src/functions/webhook.rs) (the
webhook handler + update router), and the [`src/render/`](../src/render) pipeline
(outbound streaming).

## 1. Mental model

The worker is simultaneously:

- a **Telegram Bot API client** — it POSTs to `https://api.telegram.org/bot<token>/<method>`
  for every outbound action (`sendMessage`, `editMessageText`, `setWebhook`, …); and
- an **update receiver** — it consumes inbound `Update` objects either by
  long-polling Telegram (`getUpdates`) or by receiving Telegram's HTTP POSTs
  through an engine route.

Both ingress paths converge on a single sink, so the rest of the worker never
needs to know which adapter delivered an update:

```
polling:  bg task ─ getUpdates ─┐
                                 ├─► webhook::process_update_with_tracing ─► process_update
webhook:  engine HTTP route ─────┘
```

## 2. The HTTP client

Every call goes through `api_call_with_timeout` in `telegram.rs`:

```
url   = "https://api.telegram.org/bot" + bot_token + "/" + method
client.post(url).timeout(t).json(body).send()
```

- **Client** — a shared `reqwest::Client` held in `RuntimeState.http`
  (connection pooling across calls).
- **Token** — read fresh from the hot-reloadable config snapshot
  (`deps.cfg().await.bot_token`) on every call, so a rotated token takes effect
  immediately without reconnecting.
- **Success contract** — Telegram replies `{ "ok": bool, "result"?, "description"? }`.
  The client requires `ok == true` and returns `result`; otherwise it returns
  `IIIError::Handler("telegram <method> failed: <payload>")`.
- **Token safety** — transport errors are reported via `e.without_url()` so the
  bot token (embedded in the URL) is never logged.
- **Timeouts** — 30 s default. `getUpdates` overrides this to
  `long_poll_timeout + 15 s` so the long-poll can run to completion.
- **Cancellation** — `api_call_cancellable` / `get_updates_with_cancel` race the
  request against a `CancellationToken` via `tokio::select!`, so an in-flight
  long-poll or typing ping aborts immediately on adapter switch or shutdown.

## 3. Ingress

Ingress is selected by the `updates` adapter in config (see
[configuration.md](configuration.md) for the full lifecycle). Both adapters are
supervised by `ingress::apply_updates_adapter`.

### 3.1 Polling (default)

```yaml
updates: { name: polling, config: { timeout_seconds: 50 } }
```

`run_poller` is a background Tokio task:

1. Read the current config each iteration (so `timeout_seconds` changes apply
   live, capped at Telegram's max of 50).
2. `getUpdates` with `{ offset, timeout, allowed_updates: ["message", "callback_query"] }`.
3. On success: reset backoff, process each update through
   `process_update_with_tracing`, then advance `poll_offset` to
   `last_update_id + 1` (the dedupe/acknowledge mechanism — Telegram won't
   redeliver acknowledged updates).
4. On error: warn and back off exponentially (1 s → 30 s), abortable via the
   cancellation token.

The poller's `CancellationToken` and `JoinHandle` live in `RuntimeState`; an
adapter switch or shutdown cancels the token and awaits the handle before
starting anything new. Polling needs **no public URL** — ideal for local dev.

### 3.2 Webhook (production)

```yaml
updates: { name: webhook, config: { base_url: "https://engine.example", secret: "…" } }
```

Telegram POSTs each update to the engine route `POST /telegram-bot/webhook`,
which the engine dispatches to the `telegram-bot::webhook` function. The request
arrives as an `HttpTriggerRequest { body: Value, headers: Option<Map> }`. The
handler:

1. Confirms the active adapter is `webhook` (else rejects — the route may still
   exist briefly during a switch).
2. **Validates the secret token** when one is configured: the
   `X-Telegram-Bot-Api-Secret-Token` header must equal `config.secret`
   (case-insensitive header match). When no secret is configured, the update is
   accepted with a warning — see [§8](#8-secret-validation-and-security).
3. Deserializes `body` into a `TelegramUpdate` and hands it to the shared sink.

The worker never opens its own HTTP listener — the **engine** owns the socket;
the worker only registers the route. The Telegram-facing URL is derived as
`{base_url}/telegram-bot/webhook` (see [configuration.md](configuration.md#4-the-updates-adapter-lifecycle)).

## 4. Outbound method surface

Every Telegram method the worker calls, and why:

| Method | Wrapper | Used for |
|---|---|---|
| `getUpdates` | `get_updates_with_cancel` | Polling ingress (long-poll). |
| `setWebhook` | `set_webhook` | Register the webhook URL + secret when switching to the webhook adapter (and via `set-webhook`). |
| `deleteWebhook` | `delete_webhook` | Unregister the webhook when switching to polling. |
| `setMyCommands` | `set_my_commands` | Publish the slash-command menu (`/start`, `/stop`, `/model`, `/help`, `/thinking`, `/verbosity`, `/settings`) at boot and after each config reload. |
| `sendMessage` | `send_message` | Post a new persistent message; returns its `message_id`. Carries optional `reply_markup` (keyboards) and `parse_mode`. |
| `editMessageText` | `edit_message_text` | In-place updates for the **edit** streaming transport and finalize. |
| `editMessageReplyMarkup` | `edit_message_reply_markup` | Clear an inline keyboard (e.g. after an approval is resolved). |
| `sendMessageDraft` | `send_message_draft` | Push an ephemeral live-preview draft (the **draft** transport). |
| `sendRichMessageDraft` | `send_rich_message_draft` | Stream model thinking as a rich `<tg-thinking>` draft block. |
| `sendRichMessage` | `send_rich_message` | Post a persistent rich message; returns `message_id`. |
| `answerCallbackQuery` | `answer_callback_query` | Acknowledge an inline-button tap (with optional toast text). |
| `sendChatAction` | `send_chat_action` / `_cancellable` | The "typing…" indicator (edit-transport fallback only). |
| `getFile` | `get_file` | Resolve a Telegram `file_id` to a downloadable path. |

`inline_keyboard(rows)` builds the `reply_markup` payload (rows of
`{text, callback_data}` buttons) used by the model picker and approval prompts.

### Update types handled

`allowed_updates` is restricted to `["message", "callback_query"]`:

- **message** — text/caption commands (`/start`…), plain text (→ `harness::send`),
  or media. `extract_user_content` maps a photo/document/voice with no text to a
  placeholder string (`[User sent a photo]`, etc.).
- **callback_query** — inline-button taps. The `callback_data` prefix routes the
  action: `m:` model selection, `a:` approve, `d:` deny, `w:` approve-always.

## 5. Streaming transports

Assistant output is streamed as the turn runs, using one of two transports
resolved per session (`EffectiveTransport`):

- **Draft** (preferred, Bot API 9.3+): `sendMessageDraft` pushes an ephemeral
  "the bot is composing" preview that updates in place; model thinking streams
  via `sendRichMessageDraft` as a `<tg-thinking>` block. On `turn-completed` the
  draft is **finalized** into a persistent `sendMessage`/`sendRichMessage` and the
  ephemeral draft is cleared.
- **Edit** (fallback): the first chunk is posted with `sendMessage`, then refined
  with repeated `editMessageText`. Used when drafts are unsupported.

**Per-chat auto-fallback**: if `sendMessageDraft` errors with a draft-unsupported
signal (`TEXTDRAFT_PEER_INVALID`, method-not-found, `Not Found`), the chat is
pinned to the edit transport (`draft_disabled_chats`) for the rest of its life,
and the current render falls back to edit.

See [internals.md](internals.md) for the full render state machine (revision
freshness, finalize reconciliation, per-chat message ordering).

## 6. Throttling, splitting, and the typing indicator

- **Edit throttle** — `should_edit` rejects non-increasing revisions per
  `(session, entry)` and rate-limits edits per `(chat, message)` by
  `streaming.draft_throttle_ms`.
- **Draft throttle** — `should_draft` rate-limits draft pushes per
  `(chat_id, draft_id)` by `streaming.draft_throttle_ms` (using
  `RuntimeState.draft_times`).
- **Message splitting** — `split_message` cuts text into UTF-8-safe chunks of
  ≤ 4096 bytes (`TELEGRAM_MAX_MESSAGE_LEN`); the first chunk edits/creates the
  primary message and continuations post as separate ordered bubbles.
- **Typing indicator** — only used with the **edit** transport (drafts already
  show progress). `sendChatAction(typing)` is deferred ~400 ms and re-pinged
  every ~4 s by a cancellable loop guarded by a generation counter; it is
  **suppressed** on first visible output and at `turn-completed` (Telegram has no
  "stop typing" call — each action lasts ~5 s on clients unless a bot message
  arrives).

## 7. Formatting

`format::format_outgoing` converts the LLM's markdown to the Telegram **HTML**
subset (`parse_mode: "HTML"`): bold/italic/strikethrough, inline `code` and
`pre` blocks, blockquotes, and links; lists are flattened to text, tables to
pipe-joined rows, horizontal rules to a divider. All literal text is
HTML-escaped.

Live streaming edits stay **plain text** (no `parse_mode`) until finalization, so
a partially-streamed message can never produce broken/half-open HTML markup. The
final, persisted message is the one rendered as HTML.

## 8. Secret validation and security

In webhook mode the worker validates Telegram's `X-Telegram-Bot-Api-Secret-Token`
header against `config.secret` (set with `setWebhook`'s `secret_token`):

- **secret configured** → the header must match, or the update is rejected with
  `invalid webhook secret`.
- **no secret configured** → the update is accepted, but the worker logs a
  warning recommending one. Without a secret, anyone who learns the public URL
  can inject forged updates, so a secret is strongly recommended for any
  internet-reachable deployment.

Polling mode has no such exposure — the worker initiates all contact with
Telegram, so no inbound authentication is required.

## 9. Media / files

The worker resolves media via `getFile(file_id)` and represents text-less media
to the agent with placeholder strings (see `extract_user_content`). It does not
download or re-upload binary content itself; richer media handling is delegated
to the agent stack.
