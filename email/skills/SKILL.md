---
name: email
description: >-
  Send and read email from the iii engine — SMTP send, IMAP read, and
  real-time `IDLE` push as a subscribable trigger type. Credentials live in
  `harness/auth-credentials`; this worker fetches them on every connect.
---

# email

The email worker speaks SMTP and IMAP through persistent per-account
connections. Every callable surface lives under the `email::*` namespace.
The transport is chosen per account in `config.yaml` — `provider: smtp`
for send-only accounts, `provider: imap` for two-way accounts that also
need to read and react to inbound mail.

The worker refuses to fall back to polling. If an IMAP server does not
advertise the `IDLE` capability, the supervisor fails at startup with
`E610`. Inbound messages flow through the `email::new-mail` trigger type,
fanned out by an `IDLE`-driven dispatcher the moment a server-side
`EXISTS` notification lands. Credentials never live in this worker —
every IMAP login and every `email::send` calls `auth::get_token` against
`harness/auth-credentials` under provider key `email::<account>`.

## When to Use

- An agent needs to send a transactional email right now
  (`email::send`).
- You need to enumerate configured accounts and their capabilities so an
  LLM picks the right `account` field (`email::accounts::list`).
- You need to page recent messages in an IMAP folder by UID with a stable
  cursor (`email::list`).
- You need the full parsed body of one message plus attachment part refs
  (`email::get`).
- You need to stream IMAP `SEARCH` results as NDJSON as the server
  returns matches (`email::search`).
- You need to add or remove a system flag — `\Seen`, `\Flagged`,
  `\Answered`, `\Deleted`, `\Draft` (`email::flag`).
- You need to move a message to another folder atomically with RFC 6851
  `UID MOVE` (`email::move`).
- You need to stream an attachment's raw bytes onto a response channel
  with no in-memory buffering (`email::attachment::get`).
- You need to react to inbound mail the instant it lands in a configured
  `(account, folder)` (`email::new-mail` trigger type).

## Boundaries

- Not a mail server — this is a client. Bring your own SMTP and IMAP
  endpoints.
- Not an OAuth flow runner in 0.1.0 — use app-password style credentials
  via `harness/auth-credentials` until the OAuth follow-up PR.
- Not a polling fallback — IMAP servers without `IDLE` fail fast with
  `E610` so the failure stays visible instead of silently degrading.
- Not a credential store — every connect re-fetches the secret from
  `harness/auth-credentials`. Pair the two workers; deploying `email`
  alone is unsupported.
- `email::flag` with `flag: "deleted"` does NOT expunge — use
  `email::move` to a Trash folder for visible deletion.
- `email::search` and `email::attachment::get` use a streaming response
  channel; they're invoked by sibling workers passing a
  `StreamChannelRef`, not directly from `iii trigger`.

## Functions

- `email::send` — deliver a message via the account's SMTP transport
  (STARTTLS or plain on a trusted network).
- `email::accounts::list` — enumerate configured accounts with
  `provider`, `from`, `can_send`, `can_read`, and `folders`.
- `email::list` — page recent UIDs in a folder, newest first, with
  `next_since_uid` cursor.
- `email::get` — fetch one message by UID; returns parsed `html`, `text`,
  headers, and an `attachments[]` list of part refs.
- `email::search` — stream IMAP `SEARCH` results as NDJSON frames onto
  a `StreamChannelRef`; one header-summary object per match.
- `email::flag` — add or remove a system flag on a UID via
  `UID STORE +FLAGS.SILENT` / `-FLAGS.SILENT`.
- `email::move` — move a UID to another folder with RFC 6851
  `UID MOVE`; falls back to `COPY + STORE \Deleted` when the server
  lacks `MOVE`.
- `email::attachment::get` — stream attachment bytes by `part_id` onto a
  `StreamChannelRef` chunk-by-chunk; no in-memory buffering.

SMTP send timeout is `limits.send_timeout_ms` (default 30 s). IMAP
connect timeout is `limits.imap_connect_timeout_ms` (default 15 s).
Total recipients across `to + cc + bcc` cap at `limits.max_recipients`
(default 100). Each attachment caps at `limits.max_attachment_bytes`
(default 25 MiB).

Stream-source attachments (`source.kind = "stream"`) return `E699` in
0.1.0; use `source.kind = "base64"` until the symmetric attachment-send
path lands.

## Reactive triggers

Register an `email::new-mail` trigger when a function should fire
automatically the moment a new message arrives in a configured
`(account, folder)` — without polling with `email::list`.

Reach for it when:

- A support workflow should kick off the instant a ticket email lands.
- A harness session should ingest new mail as an additional turn.
- An archival worker should mirror inbound mail into a database.

Do not bind when:

- The account is `provider: smtp` (send-only); only `provider: imap`
  accounts open an IDLE listener.
- You only need to act on outbound mail — `email::send` already gives
  you the SMTP response synchronously.

### How to bind

1. Register a handler: `registerFunction('my-worker::on-mail', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'email::new-mail',
  function_id: 'my-worker::on-mail',
  config: {
    account: 'support',
    folder: 'INBOX',
    handler_timeout_ms: 30000,
  },
})
```

Config: `account` (required, must be a configured account with an
`imap:` block), `folder` (default `"INBOX"`, must appear in
`config.accounts.<name>.imap.folders`), `handler_timeout_ms` (default
30 000).

Event payload per inbound message:

```json
{
  "account":    "support",
  "folder":     "INBOX",
  "uid":        12345,
  "message_id": "<abc@mx.example.com>",
  "from":       "alice@example.com",
  "subject":    "Ticket #42",
  "snippet":    "first ~200 chars of body",
  "ts":         "2026-05-28T10:14:00+00:00"
}
```

The dispatch is event-driven off the IMAP server's `EXISTS` push —
within milliseconds of a new message landing in the watched folder.

### Delivery semantics

Best-effort, at-most-once. The dispatcher fires `iii.trigger` per
subscriber and waits up to `handler_timeout_ms`. There is no upstream
durable queue: IMAP `IDLE` is a wakeup mechanism, not a delivery
guarantee.

- A subscriber handler that panics, times out, or returns an error is
  logged and skipped — the event is NOT redelivered.
- If the worker is down when a message lands, no `email::new-mail` event
  fires for it. On reconnect the supervisor sets the high-water mark to
  the current `UIDNEXT - 1`; UIDs that arrived during downtime are
  reachable via `email::list` but do NOT replay as events.
- For at-least-once / replay, write each `email::new-mail` event to a
  durable store from your handler (`database::execute`,
  `storage::putObject`, …) before doing meaningful work, and use
  `email::list` with a persisted `since_uid` cursor to catch up on
  downtime gaps.

## Errors

Stable across the trigger boundary. Match on the `code` field of the
returned envelope.

| Code | When |
|------|------|
| `E600` | Unknown account name |
| `E601` | `email::send` with empty `to` |
| `E602` | Total recipients over `limits.max_recipients` |
| `E603` | Account missing the required transport block |
| `E604` | `email::send` with neither `html` nor `text` |
| `E605` | Attachment over `limits.max_attachment_bytes` |
| `E606` | `auth::get_token` upstream call failed |
| `E607` | No credential stored for the account |
| `E608` | Credential payload missing `username` / `password` |
| `E609` | Address parse / MIME build failure |
| `E610` | IMAP server lacks IDLE — refusing to fall back to polling |
| `E612` | IMAP `UID SEARCH` failed |
| `E613` | Folder not in account's `imap.folders` config |
| `E614` | IMAP connect / TLS handshake failed |
| `E615` | Plain (non-TLS) IMAP refused |
| `E616` | IMAP login failed |
| `E617` | IMAP `SELECT` failed |
| `E619` | IMAP body fetch / MIME parse failed |
| `E620` | SMTP send failed |
| `E621` | Response channel close failed |
| `E622` | Unknown flag name |
| `E623` | IMAP `STORE` failed |
| `E624` | IMAP `COPY` / `STORE \Deleted` fallback failed |
| `E625` | IMAP attachment-part fetch failed |
| `E626` | Attachment payload malformed (e.g. invalid base64) |
| `E627` | `email::move` partial: copy succeeded but `STORE \Deleted` failed — message in BOTH folders, reconcile |
| `E699` | Not yet implemented in 0.1.0 |
