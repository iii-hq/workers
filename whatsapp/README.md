# WhatsApp

A Rust iii worker supporting both [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust)
linked devices and the official Meta WhatsApp Cloud API. The native WhatsApp
client, Cloud API client, encryption and SQLite storage are embedded in the executable.
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
Building the embedded ADE page also requires Node and pnpm (the release pins
Node 22.20.0 and pnpm 11.13.1). `build.rs` builds stale or missing assets; an
already-built executable needs neither tool. For reproducible frontend builds:

```sh
pnpm install --frozen-lockfile
pnpm --dir whatsapp/ui build
```

Run these from the repository root before the Cargo commands below.
`SKIP_UI_BUILD=1` uses existing `ui/dist` assets and fails if they are missing.

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
second instance. `III_URL` supplies the engine address. Resource limits and
timeouts apply live; `data_dir` and `max_events` require a worker restart.
[iii.worker.yaml](iii.worker.yaml) contains defaults.
Give each process its own `data_dir`; sharing live credential databases between
worker processes is unsupported.

The initial release target is Linux x86_64 GNU. It uses the host's standard
system libraries; a statically linked musl artifact is not part of this initial
release declaration.

## ADE interface

The running worker injects a **WhatsApp** page and configuration form into ADE.
Open `#/worker/whatsapp` on your ADE host (normally
`http://127.0.0.1:3113/#/worker/whatsapp`). The page has session creation and
selection and four tabs: **Messages**, **Groups**, **Connection** and **Cloud API**. Search
conversations, read stored messages, open participating groups and inspect
their participants and administrators. The conversation composer sends to the
selected chat only after an explicit action. Connection controls include
connect/stop, local QR rendering, phone-code pairing and recent activity.
Opening the page never starts a session or marks messages as read.

Each session keeps a separate private `inbox.db` with normalized text and media
labels from received messages, available WhatsApp history sync, and text/replies
sent through this worker. Media is not downloaded by the inbox. The archive
does not preserve view-once content or media keys and scrubs expired temporary
message bodies on archive reads and writes. `max_inbox_messages` bounds retention (default 10,000 per session);
lowering it applies immediately and removes the oldest retained messages.
Stored conversations can be read while disconnected. Participating groups and
their current membership require a connected session.

The archive starts when this worker version captures messages; it is not a
complete copy of the phone's history. **Request older messages** requests older
messages from the linked phone using an existing message as its anchor. Results
arrive asynchronously, depend on WhatsApp availability, and remain subject to
the retention limit. An empty conversation has no anchor to request history.

Use the page's settings action to edit the actual entry for this worker
instance. `whatsapp::configuration-id` resolves that ID in the worker's
namespace; the ADE host saves to the configuration service in `default`.
Environment templates are preserved by the form. Configuration changes are
re-fetched from the authoritative service; malformed reloads keep the last
valid settings and appear in `whatsapp::config-status`.

Session/media limits and both timeouts apply to subsequent operations. A lower
session limit prevents additional creation without deleting existing accounts.
Changing `data_dir` requires restart because native clients own live SQLite
credential stores; credentials are not migrated automatically. `max_events`
requires restart because it sizes both the broadcast channel and session event
rings. The page reports pending restart fields. Compose `config_override`
values remain authoritative on startup; remove an override if ADE should own
that setting. `--local-config` ignores central edits and reports local mode.

Keep all application copy, commands, accessibility labels and formatted dates
in English. User messages, group names and descriptions retain their original
content. The layout adapts to the pane width; archive details and group metadata
expand on demand. Pairing appears before recent activity, with optional connection
help at the end. The configuration form groups storage, capacity and time limits
in a single column while ADE owns saving and resetting the draft. Only settings
that require a worker restart carry a restart marker.

The page optionally loads [Cabinet Grotesk from Fontshare](https://www.fontshare.com/fonts/cabinet-grotesk),
falling back to local system fonts when offline. The font is requested directly
from Fontshare's CDN; it is not redistributed in the worker. Motion is limited
to control feedback and respects reduced-motion preferences. Content does not
animate during scrolling or message updates. QR images are generated locally.

Pairing data stays in page memory, expires, and is cleared on session change.
The activity feed is bounded; the inbox is a separate persistent archive. The send
result identifies the protocol message; receipts establish later delivery.
After a send timeout, check activity before sending again.

For UI development, run `pnpm --dir whatsapp/ui watch` and start the worker
with `III_WHATSAPP_UI_WATCH=/absolute/path/to/whatsapp/ui/dist`. Assets normally
remain embedded; no frontend server is needed in production.

## Official Meta Cloud API

The **Cloud API** tab works without a linked-device session. It supports an
existing WhatsApp Business Platform account: local readiness, an explicit
read-only connection check, existing templates, text/template sending, and a
webhook-backed message archive with delivery status. It does not register or
migrate a number, create templates, or subscribe an app automatically.

In ADE, open **Configure worker** and find **Meta Cloud API**. Enter the existing
WhatsApp Business Account ID and Phone Number ID, then the access token, Meta
App Secret, and a verify token of your choice. Enable Cloud API and save.
Cloud settings apply live. Secret inputs are masked; worker status and debug
output omit their values. ADE's configuration service remains the authority
for stored configuration. Prefer environment references in that service:

```yaml
cloud:
  enabled: true
  api_version: v26.0
  business_account_id: 'YOUR_NUMERIC_WABA_ID'
  phone_number_id: 'YOUR_NUMERIC_PHONE_NUMBER_ID'
  access_token: '${WHATSAPP_CLOUD_ACCESS_TOKEN}'
  app_secret: '${WHATSAPP_META_APP_SECRET}'
  verify_token: '${WHATSAPP_WEBHOOK_VERIFY_TOKEN}'
```

Replace both ID placeholders with digits. Environment variables must be
available to the configuration service that resolves them. A `--local-config`
YAML seed accepts literal values only. Protect files containing secrets.
Use a token authorized for the existing account: messaging requires
`whatsapp_business_messaging`; account, template and subscription inspection
also needs `whatsapp_business_management`. Follow Meta's
[getting started guide](https://developers.facebook.com/documentation/business-messaging/whatsapp/get-started)
for account permissions and production token setup.

Select **Check connection** to inspect the number, WABA membership and subscribed
apps. This is a read-only check. Readiness only describes local settings; it
does not prove the token works or the public webhook is reachable.

The worker depends on the iii HTTP worker for its callback. Use a version whose
HTTP trigger supports `public_webhook` and raw `request_body` streams (verified
with package 0.22.1). Forward a public HTTPS callback to the HTTP worker's
restricted webhook listener. The worker registers GET and POST at:

```text
/webhooks/whatsapp-cloud/<URL-encoded configuration ID>
```

Copy the exact callback path from the Cloud API tab, prefix your public HTTPS
origin, and configure it in your Meta app with the same verify token. Subscribe
the app to the WABA and enable the `messages` webhook field in the Meta dashboard.
See Meta's [webhook endpoint guide](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/create-webhook-endpoint)
and [WABA subscriptions API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-account/subscribed-apps-api).
The worker answers the plaintext verification challenge and validates
`X-Hub-Signature-256` against the exact request bytes before parsing. It accepts
only the configured WABA/phone and acknowledges matching messages after local
persistence. Repeated deliveries are deduplicated; receipts may arrive before
the send response. Missing webhook secrets or disabled Cloud API return 503.

Sending is always explicit. Text messages require an open 24-hour customer
service window and allow up to 4096 characters. Outside that window, choose an
approved template and supply its required components. **Load templates** reads
existing templates only. Destinations use international phone numbers with a
leading `+`, or supported business-scoped user IDs; native JIDs are rejected.
See [text messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/text-messages),
[templates](https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/overview)
and [business-scoped user IDs](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-scoped-user-ids).

```sh
iii trigger whatsapp::cloud::status --json '{}'
iii trigger whatsapp::cloud::check_connection --json '{}'
iii trigger whatsapp::cloud::templates::list --json '{"limit":50}'
iii trigger whatsapp::cloud::messages::send_text --json '{"chat_id":"+15550000001","text":"Hello"}'
iii trigger whatsapp::cloud::messages::send_template --json '{"chat_id":"+15550000001","name":"hello_world","language":"en_US"}'
iii trigger whatsapp::cloud::chats::list --json '{}'
iii trigger whatsapp::cloud::messages::list --json '{"chat_id":"+15550000001"}'
```

Sends make one request without automatic retry. An `accepted` result means Meta
accepted submission; later webhooks establish delivery or failure. A timed-out
send has an uncertain outcome: inspect the archive before repeating it.

Cloud archives live under `data_dir/.cloud/<phone_number_id>/inbox.db`, separate
from every native session. They retain webhook messages and worker-submitted
messages under `max_inbox_messages`; changing the phone ID selects a separate
archive. They contain message previews and media labels, without media downloads
or historical backfill. Reading them sends no read receipt. Cloud events are
available through archive polling, not the native `whatsapp::events` trigger.
The **Groups** tab continues to use the linked-device integration; this Cloud
integration does not implement Meta's separate Groups API.

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

To migrate a saved event page from an older worker while its session is stopped:

```sh
./target/release/whatsapp --local-config --config seed.yaml \
  --import-events saved-events.json --import-session work
```

The seed must point to the existing `data_dir`. This offline operation accepts
an `events::poll` response or an event array, validates session envelopes,
normalizes display fields and deduplicates messages without opening an engine
or WhatsApp connection. It prints only the processed message count. Protect and
remove the saved input after migration; it may contain private message bodies.

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
`limit`; use `whatsapp::chats::list` and `whatsapp::messages::list` for the local
message archive. `whatsapp::messages::request_history` requests an older batch
from the linked phone. Sequence reservations
survive restart, but event payloads do not. `truncated` reports unavailable
history; oversized message bodies are replaced by a bounded truncation notice.
Other oversized native events are dropped with a diagnostic.
Default retention is 1000 events per session. Individual event envelopes are
capped at 64 KiB. Callback failures are not retried automatically.
Only worker-owned diagnostics are logged, even with verbose `RUST_LOG` settings;
native protocol logs are suppressed because they can contain pairing secrets.
The executable also disables SDK trace payload capture before starting its
runtime. Timing and error spans remain available without recording function
inputs/outputs such as pairing codes and message bodies.

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
pnpm --dir ui test
pnpm --dir ui build
```

With an existing native iii engine binary, run the isolated engine integration:

```sh
III_BINARY=/absolute/path/to/iii cargo test --locked --test engine_smoke -- --ignored --nocapture
```

It starts an engine on a temporary loopback port, verifies every registered schema,
exercises offline session metadata, UI delivery and configuration reloads through
real RPC, and checks SIGTERM. It creates
no WhatsApp connection and removes its temporary state on completion.

The Cloud webhook integration additionally needs a standalone HTTP worker:

```sh
III_BINARY=/absolute/path/to/iii HTTP_BINARY=/absolute/path/to/http \
  cargo test --locked --test cloud_engine_smoke -- --ignored --nocapture
```

It starts an isolated loopback HTTP listener and verifies the exact signed body,
plaintext challenge, duplicate handling, account isolation, safe status and live
disable behavior. Test credentials are dummy values and it makes no Meta API call.

Tests cover storage/lifecycle, event bounds and projections, message construction,
addressing, media bounds, strict requests, offline registration and function
schemas. They do not pair an account or send a real WhatsApp message.
