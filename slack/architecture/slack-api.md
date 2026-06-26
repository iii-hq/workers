# slack — Slack platform notes

Facts about the Slack platform the implementation depends on.

## Auth

- **Bot token** (`xoxb-`) drives every method except search. **User token**
  (`xoxp-`) is required by `search.messages`. Identity is read from `auth.test`
  (`team`, `team_id`, `user_id`, `bot_id`, `url`, `enterprise_id`).
- **Signing secret** verifies inbound requests; it is not a token.

## Ingress: Events API over HTTP (no Socket Mode)

- Slack delivers events to a public HTTPS request URL (Events API) and
  interactivity to a separate request URL. We register both on the engine.
- Socket Mode (a worker→Slack WebSocket via `apps.connections.open` + an app
  token) is intentionally not used: it exists only to avoid a public URL, and the
  engine already provides the public HTTP surface.
- `url_verification` handshake: echo the `challenge`. Respond within 3 seconds to
  every request, then process asynchronously. Slack retries on timeout
  (`X-Slack-Retry-Num`); we dedupe on `event_id`.

## Signature verification

`v0={hex hmac_sha256(signing_secret, "v0:{timestamp}:{raw_body}")}` in
`X-Slack-Signature`, with `X-Slack-Request-Timestamp`. Verification must use the
**raw** request body (read from the engine `request_body` channel) — a
re-serialized parsed body will not match. Reject timestamps outside a 5-minute
window; compare in constant time.

## Streaming

Slack supports native assistant streaming: `chat.startStream` →
`chat.appendStream` (markdown_text chunks; `task_update` chunks for tool cards) →
`chat.stopStream`. Streaming into a channel (not a DM) requires
`recipient_user_id`. Workspaces without the AI-app feature fall back to
`chat.postMessage` + `chat.update`.

## Assistant container

`assistant_thread_started` opens the container; `assistant.threads.setStatus`
shows the thinking indicator (auto-clears on reply, 2-minute timeout);
`assistant.threads.setSuggestedPrompts` (≤4) and `setTitle` shape the thread.

## Formatting and threading

- `mrkdwn`: `*bold*`, `_italic_`, `` `code` ``, `<url|text>`, `<@U…>`, `<#C…>`. No
  tables. `markdown_text` accepts richer markdown (≤12000 chars).
- A message's `ts` is its id; replies set `thread_ts` to the parent's `ts`.
- Block Kit blocks (max 50) carry interactive buttons; clicks post a
  `block_actions` payload to the interactivity URL.

## Deprecations

- `files.upload` is sunset (2025-11-12). Use the 3-step flow:
  `files.getUploadURLExternal` → upload bytes → `files.completeUploadExternal`.
- The RTM API and the verification token are legacy; use Events API + signing.
