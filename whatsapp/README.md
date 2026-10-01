# WhatsApp

A native Rust iii worker using [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust).
The WhatsApp client, encryption and SQLite storage are embedded in the executable.
There is no Node, Chromium, sidecar process or sandbox requirement. The worker
connects to an existing iii engine; normal configuration uses the configuration worker.

The dependency is pinned to
`dd0763c4635b353c4ed52def39d030fb2f375f9a`, with `iii-sdk = 0.23.0`.
This is a native function API, not an HTTP-compatible deployment of OpenWA.
[openwa-coverage.json](openwa-coverage.json) inventories all 202 operations from
the inspected OpenWA version and distinguishes native actions, partial mappings
and missing application features.

## Build and run

Rust 1.94 or newer and a native C toolchain are needed at build time for bundled
SQLite and cryptographic dependencies. The repository release toolchain is 1.97.1.

```sh
cd whatsapp
cargo build --release --locked
./target/release/whatsapp --manifest
./target/release/whatsapp --catalog
./target/release/whatsapp --url ws://127.0.0.1:49134
```

The first two inspection commands use neither the network nor session storage.
For isolated development without the configuration worker:

```sh
./target/release/whatsapp --local-config --url ws://127.0.0.1:49134
```

Use `--config file.yaml` for a YAML seed. In normal mode, the central entry
`whatsapp` is authoritative; `III_CONFIG_NAME` selects another entry for a
second instance. `III_URL` supplies the engine address. Configuration changes
apply on restart. [iii.worker.yaml](iii.worker.yaml) contains defaults.
Give each process its own `data_dir`; sharing live credential databases between
worker processes is unsupported.

The initial release target is Linux x86_64 GNU. It uses the host's standard
system libraries; a statically linked musl artifact is not part of this initial
release declaration.

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

Sessions persist under `data_dir/<session_id>/` with private file permissions
on Unix. Opening or restarting the worker never starts account connections:
call `sessions::start` explicitly. Stop preserves credentials. Logout requests
remote unlink and removes local credentials after the native client closes;
remote unlink is best effort. Delete removes one local session without claiming
remote unlink. SIGINT/SIGTERM attempt a graceful stop and native state flush
within the shutdown deadline. A timeout can abort before flushing completes;
local credential files are preserved.

## Functions and events

`--catalog` emits the authoritative function list with JSON schemas. Families
cover sessions, messages, media, chats, groups, contacts, profile, privacy,
communities, newsletters, status, labels and call rejection/termination.
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

Delivery is best effort, with a bounded broadcast queue and per-session event
ring. Reconcile with `whatsapp::events::poll` using `after_sequence` and
`limit`; persist events externally for durable history. Sequence reservations
survive restart, but event payloads do not. `truncated` reports unavailable
history; oversized message bodies are replaced by a bounded truncation notice.
Other oversized native events are dropped with a diagnostic.
Default retention is 1000 events per session. Individual event envelopes are
capped at 64 KiB. Callback failures are not retried automatically.
Only worker-owned diagnostics are logged, even with verbose `RUST_LOG` settings;
native protocol logs are suppressed because they can contain pairing secrets.

## Limits and differences from OpenWA

- Media uses base64 with a configurable maximum, 16 MiB by default. Downloads
  accept WhatsApp CDN paths and checked cryptographic metadata, stream with a
  hard byte cap, and do not fetch arbitrary caller URLs.
- Files must already use the desired encoding. There is no voice/video
  transcoding, thumbnail generation, conversion queue or external codec process.
- Status audience recipients are explicit; the native privacy flag does not
  compute or enforce a recipient list.
- Send results contain protocol message IDs, not a delivery guarantee. Remote
  mutation timeouts can have an unknown outcome; inspect state before retrying.
- OpenWA plugins, automation rules, API-key management, webhooks administration,
  infrastructure endpoints and persisted message/chat/contact queries require
  additional application functionality. They are not advertised as working functions.
- Call rejection/termination send signaling requests. Call-link generation and
  audio/video call transport are not enabled.
- The native library is an unofficial protocol client. Its behavior and feature
  availability depend on the WhatsApp service. Offline tests do not establish
  live account interoperability or full OpenWA parity.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

With an existing native iii engine binary, run the isolated engine integration:

```sh
III_BINARY=/absolute/path/to/iii cargo test --locked --test engine_smoke -- --ignored --nocapture
```

It starts an engine on a temporary loopback port, verifies every registered schema,
exercises offline session metadata through real RPC, and checks SIGTERM. It creates
no WhatsApp connection and removes its temporary state on completion.

Tests cover storage/lifecycle, event bounds and projections, message construction,
addressing, media bounds, strict requests, offline registration and function
schemas. They do not pair an account or send a real WhatsApp message.
