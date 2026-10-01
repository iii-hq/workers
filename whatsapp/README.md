# WhatsApp

A Go iii worker supporting both [whatsmeow](https://github.com/tulir/whatsmeow)
linked devices and the official Meta WhatsApp Cloud API. The native WhatsApp
client, Cloud API client, encryption and SQLite storage are embedded in the executable.
There is no Node, Chromium, sidecar process or sandbox requirement. The worker
connects to an existing iii engine; normal configuration uses the configuration worker.

The dependency is pinned to
`8b41cfe6d9c4` (`v0.0.0-20260929112325-8b41cfe6d9c4`), with the iii Go SDK `v0.24.3`.
This is a native function API, not an HTTP-compatible deployment of OpenWA.
[openwa-rust-coverage.json](openwa-rust-coverage.json) is the historical Rust/OpenWA
comparison, not a declaration of Go support. Use `--catalog` for the supported Go API.

## Build and run

Use Go 1.27.1 on Linux. SQLite uses the pure-Go `modernc.org/sqlite` driver;
the binary builds with `CGO_ENABLED=0` and requires no C toolchain. ADE bundles
are checked in under `assets/`, so a fresh Go build needs neither Node nor pnpm.

```sh
cd whatsapp
make build
./bin/whatsapp --manifest
./bin/whatsapp --catalog
./bin/whatsapp --url ws://127.0.0.1:49134
```

The first two inspection commands use neither the network nor session storage.
For frontend changes, install dependencies from the repository root using its
Node and pnpm versions, then rebuild and copy the embedded bundles:

```sh
pnpm install --frozen-lockfile
make -C whatsapp ui
make -C whatsapp build
```

For isolated development without the configuration worker:

```sh
./bin/whatsapp --local-config --config config.example.yaml --url ws://127.0.0.1:49134
```

Use `--config file.yaml` for a YAML seed. In normal mode, the central entry
`whatsapp` is authoritative; `III_CONFIG_NAME` selects another entry for a
second instance. `III_URL` supplies the engine address. Resource limits and
timeouts apply live; `data_dir` and `max_events` require a worker restart.
[iii.worker.yaml](iii.worker.yaml) contains defaults.
Give each process its own `data_dir`; sharing live credential databases between
worker processes is unsupported.

This branch supports local Go builds. The repository release pipeline has no
Go binary recipe yet, so WhatsApp is excluded from `.deploy/workers.yaml` until
that recipe exists. No cross-platform release artifact is declared here.

### Compatibility with the Rust worker

Use the default `data/whatsapp-go` or another fresh directory. whatsmeow has its
own credential schema, and this worker rejects Rust session metadata. Stop the
old process before reusing its engine identity, then create and explicitly pair
a new Go session. Do not point this executable at the old `data/whatsapp` tree.
The old credentials and archive stay available to the Rust executable; no
automatic credential, archive, event-page or Cloud database migration is provided.
The Go CLI does not implement the Rust `--import-events` command.

An existing central configuration may still select the old directory. Update
that entry to the new directory before starting this worker, or use a separate
`III_CONFIG_NAME` for an isolated instance. Native operation coverage differs
from the Rust version; unsupported calls are omitted from registration.

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
requires restart because it sizes the native and Cloud event queues and native
session event rings. The page reports pending restart fields. Compose `config_override`
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

After editing the UI, run `make ui build` from this directory and restart the
development worker. Assets remain embedded; no frontend server is needed.

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

### Development webhook URL

In **Cloud API**, select **Start development tunnel** to obtain a complete
HTTPS callback URL, then **Copy webhook URL** and paste it into the Meta app's
webhook configuration. Use the same verify token saved in **Configure worker**,
enable the `messages` field, and subscribe the app to the WABA. Starting a
tunnel does not register or change the callback in Meta.

The `quick-tunnel` worker must run in the WhatsApp worker's namespace, with
`cloudflared` installed and its `webhooks` target pointing to the HTTP worker's
restricted webhook listener (normally `http://127.0.0.1:3112`). Only routes
explicitly marked `public_webhook` are exposed there. The generated URL is:

```text
https://<host>.trycloudflare.com/webhooks/whatsapp-cloud/<configuration ID>
```

The worker owns a separate renewable tunnel lease, so **Stop tunnel** releases
only WhatsApp's reservation. GitHub and other consumers can keep using the
same tunnel. The enabled preference is saved privately under `data_dir` and
resumes after a worker restart. Shutdown releases WhatsApp's lease; unexpected
termination leaves a bounded lease that expires unless renewed.

The page displays a callback URL only while the tunnel is ready. A recreated
Quick Tunnel can have a different hostname: copy the new URL into Meta and
verify it again. Development tunnels are temporary; use a stable HTTPS origin
for a production callback. Linked-device sessions do not need this tunnel.

The same controls are available as functions; use your deployment namespace:

```sh
iii trigger whatsapp::cloud::tunnel::start --namespace my-project --json '{}'
iii trigger whatsapp::cloud::tunnel::status --namespace my-project --json '{}'
iii trigger whatsapp::cloud::tunnel::stop --namespace my-project --json '{}'
```

Sending is always explicit. Text messages require an open 24-hour customer
service window and allow up to 4096 characters. Outside that window, choose an
approved template and supply its required components. Selecting **Approved template**
loads existing templates; text parameters have labeled fields, with **Advanced JSON**
for other components. Destinations use international phone numbers with a
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
or historical backfill. Reading them sends no read receipt. Subscribe to
`whatsapp::cloud::events` for filtered webhook notifications and reconcile with
the archive; native `whatsapp::events` subscriptions remain separate.
The **Groups** tab continues to use the linked-device integration; this Cloud
integration does not implement Meta's separate Groups API.

### Filtered Cloud webhook events

Read the running worker's registration guide with `engine::triggers::info`:

```json
{"id":"whatsapp::cloud::events","namespace":"messaging"}
```

Replace `messaging` with the WhatsApp worker's namespace. The returned
`description` explains registration, filters and payloads. Go SDK 0.24.3's
public `RegisterTriggerType` API cannot publish trigger filter/payload schemas,
so those schema fields are unavailable; runtime filter validation still applies.

Trigger type `whatsapp::cloud::events` delivers one flat event per accepted
incoming message or delivery-status object after its archive transaction commits:

```json
{
  "kind": "message.received",
  "business_account_id": "123",
  "phone_number_id": "456",
  "message_id": "wamid.example",
  "chat_id": "+15550000001",
  "sender_id": "+15550000001",
  "message_type": "text",
  "text": "Hello",
  "timestamp_ms": 1700000000000,
  "data": {
    "id": "wamid.example",
    "from": "15550000001",
    "timestamp": "1700000000",
    "type": "text",
    "text": {"body": "Hello"}
  }
}
```

`kind` is `message.received` or `message.status`. Every event includes the account
IDs, message ID, canonical chat ID and Meta's timestamp in milliseconds.
`sender_id`, `message_type`, preview `text` and `status` are present when applicable.
`message_type` retains Meta's raw type, such as `text`, `image` or `interactive`.
`data` is the original single message/status object, not the entire webhook body.
Previews normally contain at most 16 KiB. Event envelopes are capped at 64 KiB;
oversized content uses `data: {"content_omitted":true}` and a preview clipped to
8 KiB so the JSON-encoded envelope stays bounded. Handle message bodies as private data.

The strict filter accepts these fields; unknown fields are rejected:

| Field | Match |
|---|---|
| `kinds` | Event kinds; omitted or empty means only `message.received`. Set `message.status` explicitly to receive receipts. |
| `chat_ids` | Canonical international phone numbers or business-scoped user IDs. |
| `sender_ids` | Canonical sender IDs; events without a matching sender are excluded. |
| `message_types` | Exact raw Meta message types. |
| `statuses` | Exact values on `message.status` events: `sent`, `delivered`, `read` or `failed`. |
| `business_account_id` | Exact WABA ID. |
| `phone_number_id` | Exact Phone Number ID. |

List entries are alternatives (OR); populated fields combine with AND. An empty
list imposes no restriction, except for the received-only default of `kinds`.
Each list accepts at most 128 values; the complete filter is limited to 64 KiB.
Message type names use 1–64 lowercase letters, digits or underscores and start
with a letter. Account IDs are numeric strings of at most 32 digits.
Use a leading `+` for contact phone numbers and preserve business-scoped IDs such as
`BR.ENT.Abc123`. Native JIDs are not valid Cloud filters. A status event usually
has no sender or message type, so leave those filters empty for receipt subscriptions.

#### Register through an engine function

Call `engine::register_trigger` from a connection in the callback function's
namespace. For a callback `support::on-cloud-message` registered in `support`
and a WhatsApp worker in `messaging`, use a connection in `support` and this input:

```json
{
  "trigger_type": "whatsapp::cloud::events",
  "function_id": "support::on-cloud-message",
  "trigger_namespace": "messaging",
  "config": {
    "chat_ids": ["+15550000001"],
    "message_types": ["text"]
  },
  "metadata": {"workflow": "customer-support"}
}
```

`trigger_type` and `function_id` identify the trigger and callback. Supply the
Cloud filter as `config`, using `{}` for the received-only default. `metadata`
is optional and travels separately from the event payload. `trigger_namespace`
selects the trigger **provider**; the callback resolves in the **calling
connection's namespace**. When `trigger_namespace` is omitted, provider lookup
tries that connection's namespace and then `default`.

A newly connected Go SDK client must wait until `engine::workers::list` shows
its name in the intended namespace before calling this function. `Connect`
can return before worker metadata is announced; an early call can bind the
callback in `default`.

The function returns `{"id":"<generated-id>"}`. Save that ID and call
`engine::registered-triggers::info` with `{"id":"<generated-id>"}` to check
`status: "active"`, the expected `function_id`, and the returned function detail.
A `pending` registration is waiting for its trigger provider. Registration alone
does not establish that the callback function is available.

This engine-owned binding survives the caller's disconnection. Remove it with
`engine::unregister_trigger` and `{"id":"<generated-id>"}`. Removal is idempotent;
the response's `removed` field reports whether a binding existed.

#### Register through the Go SDK

With connected Go SDK clients, register the binding from the WhatsApp worker's
namespace and specify the callback's namespace explicitly. In this example,
`subscriptions` is connected in that worker's namespace and `automation` in
`support`; both clients remain connected for the subscription's lifetime:

```go
if err := automation.RegisterFunction("support::on-cloud-message", func(ctx context.Context, raw json.RawMessage) (any, error) {
    var event struct {
        Kind      string `json:"kind"`
        MessageID string `json:"message_id"`
    }
    if err := json.Unmarshal(raw, &event); err != nil {
        return nil, err
    }
    metadata, _ := iii.MetadataFromContext(ctx)
    return map[string]any{"message_id": event.MessageID, "metadata": metadata}, nil
}); err != nil {
    return err
}
filter := json.RawMessage(`{"chat_ids":["+15550000001"],"message_types":["text"]}`)
metadata := json.RawMessage(`{"workflow":"customer-support"}`)
if err := subscriptions.RegisterTriggerNamespaced(
    "support-cloud-messages", "whatsapp::cloud::events",
    "support::on-cloud-message", "support", filter, metadata,
); err != nil {
    return err
}
```

The callback receives binding metadata through `iii.MetadataFromContext`,
separately from the event payload. Callback results do not send WhatsApp messages;
outbound actions still require an explicit call to a Cloud send function.

Webhook deduplication is persistent in the Cloud archive's SQLite database and
commits with the message/receipt changes before publication. The latest
`5 * max_inbox_messages` deduplication entries are retained and cleared with the
archive. Within that retained history, repeated messages and repeated status
states publish once. A distinct late status still publishes once even if it
has a lower rank than the status already shown in the archive. Local sends do
not emit `message.received`; their subsequent Meta webhook receipts can emit
`message.status`.

Delivery is best effort through a separate Cloud queue bounded by `max_events`.
Queue overflow, process shutdown or callback failure can lose a notification;
there is no outbox, callback retry or Cloud event polling endpoint. Deduplication
does not guarantee callback delivery. Reconcile application state with
`cloud::chats::list` and `cloud::messages::list`; the archive records current
retained state rather than every historical receipt transition. Cloud configuration
changes invalidate queued events from the previous configuration.

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

## Validation

```sh
make test
go vet ./...
go test -race ./...
pnpm --dir ui test
make ui build
```

Tests cover storage/lifecycle, event bounds and projections, message construction,
addressing, media bounds, strict requests, offline registration and function
schemas. They do not pair an account or send a real WhatsApp message.
`make catalog` prints the registered schemas. After changing `catalog.json`, run
`make generate` to refresh the SDK schema adapters and rerun tests.

Go SDK 0.24.3 cannot advertise trigger filter or event payload schemas to the
engine. Runtime filter validation remains active; [Functions and events](#functions-and-events)
defines the native subscription contract and [Filtered Cloud webhook events](#filtered-cloud-webhook-events)
defines the Cloud contract.

The optional engine smoke test starts an isolated engine and worker on loopback
with temporary configuration and storage. Point it at existing local binaries:

```sh
go build -o /tmp/whatsapp-go-smoke .
III_BINARY=/path/to/iii \
WHATSAPP_BINARY=/tmp/whatsapp-go-smoke \
HTTP_BINARY=/path/to/http \
go test -run '^TestEngineSmoke$' -count=1 -v .
```

This checks central configuration reloads, runtime schemas, ADE assets, offline
session lifecycle and event delivery across namespaces. `HTTP_BINARY` is optional;
when supplied, it also checks webhook verification, raw-body signatures, account
isolation, duplicate message persistence, filtered Cloud callbacks, callback
namespace/metadata and archive visibility before callback delivery using synthetic
credentials and data.
A synthetic quick-tunnel provider checks the development controls, shared lease
isolation, namespace routing and callback changes through the real engine; it
does not launch cloudflared or expose the test server publicly.
The test does not pair an account or call Meta. Without `III_BINARY` and
`WHATSAPP_BINARY`, it is skipped.
