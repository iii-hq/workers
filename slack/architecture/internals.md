# slack — internals

## API surface

Each typed `slack::*` function serializes its request struct into Slack API
params and POSTs to `https://slack.com/api/<method>` with the bot token (or user
token for `search.messages`). The full Slack payload is returned wrapped in the
typed `SlackResponse { ok, ..data }`. `slack::call { method, params }` reaches any
method without a typed wrapper. A non-`ok` Slack response becomes a handler error.

## Inbound bridge

### Ingress (HTTP only)

The bridge registers two engine HTTP routes when `public_base_url` +
`signing_secret` are set:

- `slack/events` → `slack::events`
- `slack/interactions` → `slack::interactions`

Both are channel-based handlers: they read the **raw** request body from the
engine `request_body` read channel (`httpio::read_incoming`), verify the Slack v0
signature over those exact bytes (`signing::verify`, 5-minute replay window,
constant-time compare), then write the response on the paired write channel. The
raw body is required because the signature is computed over it; a re-serialized
parsed body would not match. Socket Mode is intentionally not used — the engine's
HTTP triggers are the public surface.

`slack::events` answers the `url_verification` challenge, acks `event_callback`
within Slack's 3-second window, and processes the event asynchronously.

### Triggering and context

`dispatch.rs` gates turns:

- `app_mention` (channel) or any `message.im` (DM) → run a turn.
- A channel `message` that does **not** mention the bot is captured into a
  per-thread pending buffer in `iii-state` and never acted on alone.
- `event_id` dedupe guards Slack retries.

`turn.rs` builds the turn. On the first mention in a thread it backfills prior
replies (`conversations.replies`) plus the pending buffer into the session as
context, bounded by `backfill_max_messages`. The model comes from `default_model`
(validated against `router::models::list`) or the first available model — no
vendor names are baked in. The Slack channel-context prompt
(`prompts/channel-context.txt`) is layered onto the system prompt. The returned
`session_id` is mapped to the thread so streamed output knows where to go.

### Streaming back

`session::message-*` events for the bound session reach `stream.rs`, which opens
`chat.startStream` once, sends each text delta via `chat.appendStream`, and closes
with `chat.stopStream` on `harness::turn-completed` (with a `chat.update` fallback
for workspaces without AI-app streaming). The assistant-thread "thinking" status
is set best-effort via `assistant.threads.setStatus`.

### Approvals

When the optional `approval-gate` worker is present, `approval::pending-created`
posts a Block Kit message with Approve / Reject / Approve-always buttons; the
`block_actions` interaction resolves the held call via `approval::*` and clears
the buttons.
