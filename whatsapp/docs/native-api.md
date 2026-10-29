# Use linked-phone functions and events

Pair a native WhatsApp session, send a message, subscribe to session events, and understand the supported API boundaries. This transport uses whatsmeow and stays separate from Meta Cloud API.

## Pair and send

With the worker registered in a running engine:

```sh
iii trigger whatsapp::sessions::create --json '{"session_id":"work"}'
iii trigger whatsapp::sessions::start --json '{"session_id":"work"}'
iii trigger whatsapp::sessions::qr --json '{"session_id":"work"}'
iii trigger whatsapp::sessions::get --json '{"session_id":"work"}'
```

Render the returned QR `code` as a QR image and scan it from WhatsApp's linked
devices screen, or request `sessions::pair_code` with `phone_number`.
Pairing material expires and is returned only by these dedicated functions.
A null QR means none is currently available. Wait for the `connected` state
before sending.

```sh
iii trigger whatsapp::messages::send_text --json '{"session_id":"work","chat_id":"+15550000001","text":"Hello"}'
iii trigger whatsapp::sessions::stop --json '{"session_id":"work"}'
```

Send only to an intended, authorized recipient. Phone numbers normalize to
`@s.whatsapp.net`; legacy OpenWA `@c.us` also normalizes. LIDs remain opaque
`@lid` addresses. Group and newsletter functions validate their specific JID
types.

Session metadata persists under `data_dir/<session_id>/` with private file
permissions on Unix. Native keys and messages persist in the database worker. Opening or restarting the worker never starts account connections:
call `sessions::start` explicitly. Stop preserves credentials. Logout requests
remote unlink and clears persisted credentials after the native client closes;
remote unlink is best effort. Delete clears one session and its owned database rows without claiming
remote unlink. SIGINT/SIGTERM attempt a graceful stop and native state flush
within the shutdown deadline. A timeout can abort before flushing completes;
persisted credentials are preserved until the stop completes.

## Functions and events

`--catalog` emits the authoritative function list with JSON schemas. Families
cover sessions, messages, media, chats, groups, contacts, profile, privacy,
communities, newsletters, labels, call rejection and the official Cloud API.
Inputs are strict: unknown fields fail. WhatsApp feature operations require a
connected session; connection and pairing use the session lifecycle functions.
Server/app-state errors are returned, including participant
errors within otherwise successful group responses.

Trigger type `whatsapp::events` accepts:

```json
{"session_id":"work","kinds":["message.received","session.connected"]}
```

Omit `session_id` for all sessions; an empty `kinds` list accepts all event kinds.
Payloads contain `session_id`, `sequence`, `kind`, `timestamp_ms`, and `data`.
Callbacks preserve the binding's namespace and metadata. QR/pairing events
announce availability/expiry and do not contain the pairing secret.

`message.received` data contains `info`, `message` and `kind`. `info` holds
`id`, `timestamp_ms`, `push_name`, `edited` and `source` (`chat`, `sender`,
`sender_alt`, `is_from_me`, `is_group`, `addressing_mode`; `sender_alt` and
`addressing_mode` are `null` when absent). A revoke reports the original author
as `sender`; when someone else revoked it (a group admin), `sender_alt` is `null`
and `push_name` is empty, because the envelope's values are the admin's.

`message` holds `conversation`, `mentioned_jids`, `mentions_me`,
`mentioned_me_jids`, `file_name` and `quoted`. `mentions_me` matches any
mentioned JID, device ignored, against the linked account's phone JID or LID;
`mentioned_me_jids` lists the entries that matched. `file_name` is the message's
own document file name (empty for other kinds); a document without a caption
repeats it in `conversation`. `quoted` is `null` unless the message replies to
another one, and otherwise holds its `id`, `participant`, `is_me`, `kind`, `text`
(at most 4096 bytes) and `file_name` (empty unless it is a document). A reply
whose quoted message was not sent along (a stanza ID without the message) still
gets `quoted`, with `kind` `unknown` and empty `text` and `file_name`.

View-once and disappearing messages expose no text, file name, mentions or
quote. A quoted view-once message has `kind` `view_once`; a quoted disappearing
one keeps its kind with empty `text` and `file_name`.

Delivery is best effort, with a bounded broadcast queue and per-session event
ring. Reconcile with `whatsapp::events::poll` using `after_sequence` and
`limit`; use `whatsapp::chats::list` and `whatsapp::messages::list` for the local
message archive. `whatsapp::messages::request_history` requests an older batch
from the linked phone. Sequence reservations
survive restart, but event payloads do not. `truncated` reports unavailable
history; oversized event data is replaced by `{"content_omitted":true}`.
Default retention is 1000 events per session. Individual event envelopes are
capped at 64 KiB. Callback failures are not retried automatically.
Only worker-owned diagnostics are logged; native protocol logs are suppressed
because they can contain pairing secrets.
The executable also disables SDK trace payload capture before starting its
runtime. Timing and error spans remain available without recording function
inputs/outputs such as pairing codes and message bodies.

## Limits and differences from OpenWA

- Media uses base64 with a configurable maximum, 16 MiB by default. Downloads
  accept WhatsApp CDN paths and checked cryptographic metadata, stream with a
  hard byte cap, and do not fetch arbitrary caller URLs.
- Files must already use the desired encoding. There is no voice/video
  transcoding, thumbnail generation, conversion queue or external codec process.
- Status publishing is excluded: whatsmeow uses account-wide privacy settings
  and cannot preserve the original API's explicit per-send recipient list.
- Send results contain protocol message IDs, not a delivery guarantee. Remote
  mutation timeouts can have an unknown outcome; inspect state before retrying.
- OpenWA plugins, automation rules, API-key management, webhooks administration,
  infrastructure endpoints and persisted message/chat/contact queries require
  additional application functionality. They are not advertised as working functions.
- Call rejection sends signaling only and requires `peer` and `call_creator`
  to identify the same person. Call termination, call links and audio/video
  call transport are not exposed.
- The native library is an unofficial protocol client. Its behavior and feature
  availability depend on the WhatsApp service. Offline tests do not establish
  live account interoperability or full OpenWA parity.

The following original native IDs are intentionally not registered. Each uses
the `whatsapp::` prefix:

- `status::audience`, `status::send_text`, `status::send_media`, `status::revoke`;
  `calls::terminate`; `chats::unsubscribe_presence`.
- `newsletters::remove_picture`, `newsletters::delete`, `newsletters::update`,
  `newsletters::set_picture`, `newsletters::change_owner`, `newsletters::demote_admin`,
  `newsletters::admin_info`, `newsletters::followers`,
  `newsletters::mute_follower_activity`, `newsletters::vote_poll`, `newsletters::my_addons`.
- `groups::remove_linked_participants`, `groups::cancel_requests`,
  `groups::revoke_request_codes`, `groups::set_history`,
  `groups::set_forwarding_restriction`, `groups::set_admin_reports`,
  `groups::set_limit_sharing`, `groups::set_member_link_mode`, `groups::set_member_history_mode`.
- `contacts::remove`, `privacy::update_disallowed`, `communities::deactivate`,
  `communities::join_subgroup`, `communities::remove_participants`.

Unavailable group/contact/newsletter metadata is returned as `null`, with
matching response schemas, rather than guessed values. Native send results
have `recipient_fanout: null` because whatsmeow does not expose those counters.
