# Native WhatsApp worker

The user selected `whatsapp-rust` after evaluating a Node wrapper and requires
a Rust executable without Node, a browser, Docker or a microVM. The original
OpenWA API remains the coverage reference; compatibility must be measured and
documented, never implied by function registration alone.

## Architecture

Add `whatsapp/` as a standalone Rust workspace, binary and iii worker identity.
Link `iii-sdk = 0.23.0` and `whatsapp-rust` at immutable revision
`dd0763c4635b353c4ed52def39d030fb2f375f9a`. Functions use `whatsapp::*` ids.
Do not launch subprocesses or expose an HTTP server. Register the full native
function catalog before any connection to WhatsApp; creating or starting a
session is an explicit function call. No account or message is touched on boot.

`SessionManager` owns persistent named sessions, native clients and run tasks. One
SQLite credential database per session avoids session mixing. Session ids are
restricted to safe ASCII path components. Stop preserves credentials; logout
requests best-effort unlink through the native API; removal must stop first and affect only that
session. Credentials and QR codes never enter logs. Session summaries omit
secrets; a dedicated pairing function returns the current expiring QR.

`Registry` owns typed request/response schemas and handlers. It installs them
in iii and exposes offline catalog/invocation seams for tests. Request validation
rejects invalid JIDs, empty/oversize text, invalid media and unknown fields.
Common request conventions use `session_id`, `chat_id` or `group_id`; numbers
and OpenWA `@c.us` ids normalize to native PN JIDs without confusing `@lid`.

Native groups, contacts, profile, privacy, messaging, media, chats, communities,
newsletters, status and labels are exposed through explicit adapters. Preserve
upstream errors; do not blindly retry non-idempotent sends. Upstream types without
Serde are projected into meaningful DTOs; never substitute Debug strings for
structured data. Unsupported OpenWA administration/plugin operations are recorded
as coverage gaps, not registered as handlers that always report success.

Events have a session id, monotonic sequence and stable kind. Keep bounded
per-session queues for polling and deliver a typed `whatsapp::events` trigger.
Pairing and lifecycle snapshots remain readable even without event subscribers.
Long-lived tasks are stopped on SIGINT/SIGTERM. Inbound events are observations;
no automatic replies or harness integration are enabled.

## Delivery and verification

Ship configuration, manifest, permissions, README, skill, catalog, coverage matrix,
locked dependencies and release catalog metadata. Support `--manifest` and
`--catalog` without a running engine or WhatsApp connection. Use Cargo for Rust
checks. Any repository Python validation runs inside the template test container.
Test typed catalogs, validation, session persistence, duplicate lifecycle calls,
shutdown and handler failures offline. Verify an actual binary and iii registration
with an isolated engine when available. Live WhatsApp success requires a user
pairing a device and is reported separately from offline validation.

## Scope

This delivery prioritizes the selected native client and a complete OpenWA coverage
inventory. Exact OpenWA administration compatibility requires additional application
functionality. JavaScript plugin execution conflicts with the no-Node design and
is not introduced implicitly.
