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

For Meta Cloud API tasks, use `whatsapp::cloud::*` without a native session. Inspect `cloud::status`; run `cloud::check_connection` only for a requested connection test. Use phone numbers or business-scoped IDs accepted by that function's schema. Send only the authorized text or approved template to the selected recipient. Cloud groups, account provisioning and historical backfill are not implemented. Read captured messages through the Cloud archive functions; native event subscriptions do not deliver Cloud events. Consult the README's Cloud API section when configuring credentials or webhooks.
