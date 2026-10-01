---
name: whatsapp
description: Manage native WhatsApp sessions, send authorized messages or media, operate groups and contacts, and consume WhatsApp events through the iii whatsapp worker.
---

# WhatsApp

1. Discover the relevant `whatsapp::*` function and inspect its schema. The native contract differs from OpenWA; [coverage](../openwa-coverage.json) records mapped actions and gaps.
2. Use the user's selected session. `sessions::create` creates offline metadata; `sessions::start` begins connecting. After restart, start explicitly again. Pair through `sessions::qr` or `sessions::pair_code` when required, sharing pairing material only with the account owner.
3. Subscribe to trigger type `whatsapp::events` with `session_id` and exact `kinds` before operations that need follow-up. Preserve callback namespace and metadata. Read `sessions::get` once after subscribing to close races; use engine wake/event mechanisms for ongoing work.
4. Call outbound functions only when the session is connected and the user has authorized the recipient and action. Use string JIDs returned by the worker; preserve `@lid` identities, including device suffixes. Never infer a telephone number from a LID.
5. Media functions accept bounded base64 bytes. Prepare the actual requested format before sending: the worker performs no transcoding or arbitrary URL fetch. Status sends require explicit recipients; a privacy mode alone does not filter the audience.
6. Retain returned message IDs and event sequences. A send response is not a delivery/read receipt. A timeout can leave an unknown remote outcome: inspect events/state before retrying a mutation.
7. Event delivery is best effort. Reconcile gaps with `whatsapp::events::poll`; `truncated` means some history is unavailable. Persist events in an application if durable history is needed.
8. `sessions::stop` preserves credentials. `sessions::logout` requests remote unlink and clears local credentials; remote unlink is best effort. `sessions::delete` removes local state. Perform these disruptive actions only within the user's request.

Return factual operation results and per-participant failures. Treat pairing codes, QR payloads, media keys and message content as sensitive; keep them out of logs. See [README](../README.md) for deployment, resource limits and the complete offline function catalog command.
