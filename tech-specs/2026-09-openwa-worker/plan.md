# Native WhatsApp worker implementation plan

**Goal:** a usable iii Rust worker with the native WhatsApp client embedded.
**Spec:** [design.md](design.md).
**Architecture:** typed function adapters share a native session manager and a
registry. The manager owns durable sessions, pairing and event lifecycle.
**Tech stack:** Rust, Tokio, iii-sdk 0.23.0, pinned whatsapp-rust, bundled SQLite.

## Global constraints

- No Node, subprocess runtime, browser, Docker or sandbox at runtime.
- No WhatsApp connection at worker startup or during interface capture.
- Preserve existing workspace edits; development uses an isolated worktree.
- Cargo checks run natively; Python validation uses the template container.
- Do not claim all OpenWA operations work without per-operation evidence.

## Tasks

- [x] 1. Create package, shared registry and validation. Tests: stable unique
  catalog ids and schemas, malformed payloads, native/openwa JIDs, bounded media.
- [x] 2. Implement persistent session lifecycle, pairing, native event capture,
  polling and clean shutdown. Tests: unsafe ids, duplicate create/start, stop,
  remove, persistence across manager reopen, expired QR, bounded event queues.
- [x] 3. Implement groups/contacts/profile/privacy/community function adapters.
  Tests: input validation and correct transformations without a live account.
- [x] 4. Implement messages/media/chat/newsletter/status/label adapters. Tests:
  protobuf construction, media type/size bounds, explicit status recipients,
  correct message keys and required session readiness.
- [x] 5. Wire iii configuration, event triggers and CLI; ship docs, permissions,
  skill and release metadata. Build an OpenWA route coverage inventory.
- [x] 6. Run fmt, clippy, tests and binary smoke tests; collect actual iii interface
  with a local isolated engine if available; request independent review and fix
  material issues. Bring verified new files into the user's working checkout.

## Shared interfaces

`Registry::add::<Req, Resp, _, _>(id, description, handler)` captures typed schemas
and a handler `Fn(Arc<SessionManager>, Req) -> Future<Result<Resp>>`.
`SessionManager::client(&str) -> Result<Arc<whatsapp_rust::Client>>` requires a
connected session. `SessionManager::config()` returns `&Config`.
`validate::jid(&str) -> Result<Jid>` normalizes user input;
`validate::text(&str, field: &str, max: usize) -> Result<()>` validates text;
`validate::media(&str, max_bytes: usize) -> Result<Vec<u8>>` decodes bounded base64.
Adapters export `pub fn register(registry: &mut Registry)` and own their typed
request/response structs. Root owns registry/config/bootstrap, session agent owns
sessions, group agent owns groups/contact/profile/privacy/community, message agent
owns messages/media/chat/newsletter/status/labels. No file has two implementers.

## Review focus

- Concurrent start/stop/delete must not leak or resurrect a client.
- Pairing expiry and disconnect must invalidate cached readiness/QR state.
- No operation interprets an unmapped LID as a phone number.
- Media decoding and event queues must remain bounded.
- A stopped or absent session fails before any outbound operation.

## Progress

Initial repository is dirty; isolated branch `feat/whatsapp-native` starts at
`8c72e7dc0`. User approval covers proceeding with the selected native architecture;
routine implementation choices do not require another approval round.

The implemented catalog contains 145 typed functions and one typed event trigger.
The 202-operation OpenWA inventory records 71 native equivalents, 23 partial
mappings and 108 unimplemented operations; these classifications do not claim
drop-in API compatibility or live WhatsApp verification.

Independent review corrected pairing-secret logging, shutdown lock deadlines,
stale pairing callback invalidation, string JID projections, chat timestamp units,
group reaction keys, poll destination validation and pre-upload media validation.
Each behavior change has an offline regression test. Runtime code launches no
subprocesses, and the Linux release binary links only the standard system libraries.

The user subsequently requested keeping the implementation in its worktree and
committing it locally. The research note, source, tests, documentation and registry
metadata belong to `feat/whatsapp-native`; duplicate implementation files are
removed from the original checkout while unrelated changes are preserved.
