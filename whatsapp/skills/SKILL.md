---
name: whatsapp
description: Use the WhatsApp worker to manage linked-device sessions, send authorized messages, operate groups, consume events, or use an existing Meta Cloud API account.
---

# WhatsApp

1. Discover the relevant registered `whatsapp::*` function and inspect its schema. `--catalog` lists implemented Go functions; [README](../README.md#limits-and-differences-from-openwa) explains unavailable operations and nullable metadata. Use the current catalog when choosing an operation.
2. Choose the integration requested by the user. Linked devices use whatsmeow and the selected `session_id`: `sessions::create` creates offline metadata; `sessions::start` connects explicitly after creation or restart. Pair through `sessions::qr` or `sessions::pair_code`, sharing pairing material only with the account owner. Go sessions need fresh storage and pairing; see the README before replacing a Rust instance.
3. Subscribe to trigger type `whatsapp::events` with `session_id` and exact `kinds` before operations that need follow-up. Preserve callback namespace and metadata. Read `sessions::get` once after subscribing to close races; use engine wake/event mechanisms for ongoing work.
4. Call outbound functions only when the session is connected and the user has authorized the recipient and action. Use string JIDs returned by the worker; preserve `@lid` identities, including device suffixes. Never infer a telephone number from a LID.
5. Media functions accept bounded base64 bytes. Prepare the requested encoding before sending; the worker performs no transcoding or arbitrary URL fetch. Status publishing is unavailable in this Go version.
6. Retain returned message IDs and event sequences. A send response is not a delivery/read receipt. A timeout can leave an unknown remote outcome: inspect events/state before retrying a mutation.
7. Event delivery is best effort. Reconcile gaps with `whatsapp::events::poll`; `truncated` means some history is unavailable. Use `chats::list` and `messages::list` for locally retained messages without read receipts. Coverage describes retained history, not the phone's complete archive.
8. `sessions::stop` preserves credentials. `sessions::logout` requests remote unlink and clears local credentials; remote unlink is best effort. `sessions::delete` removes local state. Perform these disruptive actions only within the user's request.

Return factual operation results and per-participant failures. Treat pairing codes, QR payloads, media keys and message content as sensitive; keep them out of logs. See [README](../README.md) for deployment, resource limits and the complete offline function catalog command.

## Meta Cloud API

Use `whatsapp::cloud::*` without a native session. Inspect `cloud::status`; run
`cloud::check_connection` for a requested connection test. Use canonical phone
numbers with a leading `+` or business-scoped user IDs. Send only the authorized
text or approved template to the selected recipient. Cloud groups, account
provisioning and historical backfill are unavailable. For credentials or webhook
setup, read [Official Meta Cloud API](../README.md#official-meta-cloud-api).

For ongoing Cloud webhook work, subscribe to `whatsapp::cloud::events` in the
worker's namespace, preserving the callback namespace and binding metadata.
Read [Filtered Cloud webhook events](../README.md#filtered-cloud-webhook-events)
before constructing the filter or consuming its payload. Empty or omitted
`kinds` means only `message.received`; opt into `message.status` explicitly.
Use the documented account, chat, sender, message-type and status filters to
limit callbacks to the requested scope. These events carry the original single
Meta object in `data` and are separate from native session events.

After subscribing, read `cloud::chats::list` and `cloud::messages::list` to
reconcile retained state and registration gaps. Repeat reconciliation after a
connection gap or callback failure: Cloud delivery is best effort, deduplicated
and bounded, with no callback retry or event polling endpoint. An archive is
current retained state, not a receipt-transition log. Local sends do not emit
`message.received`; use later webhook statuses to establish delivery, and inspect
the archive before retrying a send with an uncertain outcome.
