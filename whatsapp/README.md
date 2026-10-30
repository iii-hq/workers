# whatsapp

Send and receive WhatsApp messages through iii functions using linked phones or the official Meta WhatsApp Cloud API. Pair accounts, inspect conversations and groups, send attachments and templates, and observe filtered events. An ADE page helps you configure and test each transport separately. The worker runs as a Go binary without Node.js or Chromium.

## Install

Install the worker into your existing iii Compose project:

```bash
iii call compose::add worker=whatsapp
```

Compose resolves the declared `database`, `configuration`, `http`, and `quick-tunnel` dependencies, updates `worker-compose.yaml`, and starts the project. Dependency installation does not create the dedicated WhatsApp database or configure a Meta account. Configure storage before the worker's first successful start, as described under [Configuration](#configuration).

Release binaries support Linux and macOS on x64 and arm64. Windows is not supported. Cloudflared is needed only when you start a development webhook tunnel; linked phones do not need it.

## Quickstart

Choose the transport that matches your account:

| Transport | Connect with | Use it for |
| --- | --- | --- |
| Linked phone | A QR code or phone pairing code | Named sessions, phone conversations, participating groups, and native events |
| Meta Cloud API | An existing WhatsApp Business Platform account | Official business messaging, approved templates, attachments, and webhook events |

Examples use the `default` namespace. Add `--namespace your_namespace` when the worker runs elsewhere. Replace the example recipient with a phone you control before sending.

### Linked phone

Create a session named `support`, start it, and link your phone. In ADE, open **WhatsApp → Linked phone → Connection** and scan the QR code from WhatsApp's **Linked devices** screen. Without ADE, follow the [pairing function walkthrough](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md#pair-and-send).

Once the session reports `connected`, send a text message:

```bash
iii call whatsapp::messages::send_text --json '{
  "session_id": "support",
  "chat_id": "+15551234567",
  "text": "Hello from iii."
}'
```

Example result, with an illustrative message ID:

```json
{
  "chat_id": "15551234567@s.whatsapp.net",
  "message_id": "3EB0EXAMPLE",
  "recipient_fanout": null
}
```

Stopping a session preserves its credentials. Starting the worker does not automatically connect saved sessions. Native group operations require a connected session.

### Meta Cloud API

Configure your business account and webhook using the [Cloud setup guide](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md). Send a message from your test phone to the business number to open the customer service window, then reply through the official transport:

```bash
iii call whatsapp::cloud::messages::send_text --json '{
  "chat_id": "+15551234567",
  "text": "Hello from iii Cloud API."
}'
```

Example result, with an illustrative message ID:

```json
{
  "chat_id": "+15551234567",
  "message_id": "wamid.example",
  "status": "accepted",
  "message_status": null
}
```

`accepted` means Meta accepted the request. Delivery and read status arrive later through webhooks. Outside the customer service window, explicitly choose an approved template. Sends are attempted once; a timeout can leave the outcome unknown, so inspect the conversation before sending again.

## Configuration

Edit **WhatsApp → Configure worker** in ADE, or edit the worker's entry in the `configuration` service. The default entry ID is `whatsapp`; `III_CONFIG_NAME` selects a different one. Persisted entries use the configuration service's storage adapter, normally `data/configuration/<entry_id>.yaml`. In normal mode, the worker does not maintain a separate live configuration file.

### Configure storage first

Add a dedicated connection named `whatsapp` to the database worker's existing `databases` map. Keep unrelated entries intact:

```yaml
databases:
  whatsapp:
    url: sqlite:./data/whatsapp.db  # Path on the database worker's host
    pool:
      max: 1                     # Required by the WhatsApp storage adapter
```

The adapter requires SQLite 3.41 or later; PostgreSQL and MySQL are not supported. Ensure the parent directory exists and restrict access to the database: it contains linked-device keys and message archives. Restart WhatsApp after the database becomes available. Follow the [storage and migration guide](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/database-storage.md) for deployment permissions, backups, and existing data.

Set these fields on the WhatsApp worker:

```yaml
database:
  name: whatsapp                 # Connection in the database worker
  namespace: ''                  # Empty uses the WhatsApp namespace
data_dir: data/whatsapp-go        # Private session metadata directory
device_name: ''                   # Name in the phone's Linked devices; empty shows "Other device"
device_platform: desktop          # Platform type reported with device_name
max_inbox_messages: 10000         # Retained messages per archive
max_media_bytes: 16777216         # 16 MiB worker transfer limit
operation_timeout_secs: 30        # Deadline for subsequent operations
cloud:
  enabled: false                 # Linked phones need no Meta credentials
```

Native keys and both transports' archives persist through the database worker. Give each WhatsApp process a distinct `data_dir` and configuration identity; keep that identity and its namespace stable across upgrades. Limits and timeouts apply live; `database`, `data_dir`, `device_name`, `device_platform`, and `max_events` require a restart. `device_name` and `device_platform` reach WhatsApp only when a device pairs: an already linked device keeps its name and icon until it is unlinked and its QR code is scanned again. Before downgrading to a version without these keys, remove them from the configuration entry, since older versions reject unknown keys. Invalid reloads keep the last valid settings; `whatsapp::config-status` reports the outcome and pending restart fields. Changing the database target does not migrate existing data.

Other fields and defaults are documented in the [configuration schema](https://github.com/iii-hq/workers/blob/main/whatsapp/config.schema.json).

### Configure Meta credentials

Under **Meta Cloud API**, enable the integration and enter the WhatsApp Business Account (WABA) ID, Phone Number ID, access token, app secret, and a verify token you choose. The verify token must match Meta's webhook settings. Phone Number ID is a Meta identifier, not the phone's digits.

You can supply these fields through the WhatsApp process environment instead:

```dotenv
WHATSAPP_CLOUD_ENABLED=true
WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID=123456789012345
WHATSAPP_CLOUD_PHONE_NUMBER_ID=123456789012346
WHATSAPP_CLOUD_ACCESS_TOKEN=your_access_token_here
WHATSAPP_META_APP_SECRET=your_app_secret_here
WHATSAPP_WEBHOOK_VERIFY_TOKEN=your_webhook_verify_token_here
```

Set actual credentials in your deployment environment or a private, uncommitted `env_file`. Defined variables override saved fields at runtime, including explicit empty values; values are never written back. Restart after changing the process environment. ADE shows overridden fields without revealing their values. See [environment configuration](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md#worker-environment-configuration) for precedence, Graph API version, and central environment references.

## Official Meta Cloud API

Use **Meta Cloud API** in ADE to check the connection, test text or templates, send attachments, and inspect the local Cloud archive. Configure a public HTTPS callback and subscribe the Meta app to the WABA and `messages` field. Local readiness does not prove remote authentication or webhook reachability.

For development, **Webhook setup → Start development tunnel** creates a callback URL through `quick-tunnel`. Enable the HTTP worker's restricted webhook listener, install cloudflared on the tunnel worker's host, and point its `webhooks` target at that listener. Copy the URL into Meta with the matching verify token; starting a tunnel does not register it with Meta. Temporary URLs can change after a tunnel restart. See the [webhook walkthrough](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md#development-webhook-url).

### Upload and send documents

Upload with `whatsapp::cloud::media::upload_document`, then reuse the returned `media_id` with `whatsapp::cloud::messages::send_document`. Markdown (`.md` or `text/markdown`) is rejected; explicitly choose a supported TXT or PDF file. The worker does not rename or convert it. Follow the [document upload and send examples](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md#upload-and-send-documents).

See [Cloud API coverage and contracts](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-api.md) for media, interactive messages, account management, Flows, and eligible group, calling, and commerce operations. Meta controls account eligibility and permissions. Cloud groups use their own IDs and functions, separate from native linked-phone groups.

## Event functions

The worker hosts two event sources. Each event is one call of the function, which returns the event, and iii delivers it to the function's observers. Both are private: only this worker calls them, and anyone may observe them. Neither takes args; observers select events with filters on the event's fields. The worker calls each function only while something observes it.

| Function | Args | Filter on | Event |
| --- | --- | --- | --- |
| `whatsapp::on-events` | none | `input.session_id`, `input.kind` | `{ session_id, sequence, kind, timestamp_ms, data }`: native session, message, receipt and group events |
| `whatsapp::cloud::on-events` | none | `input.kind`, `input.chat_id`, `input.sender_id`, `input.message_type`, `input.status`, `input.business_account_id`, `input.phone_number_id`, `input.field`, `input.event`, `input.group_id`, `input.participant_id` | `{ kind, business_account_id, phone_number_id, message_id, chat_id, sender_id?, message_type?, text?, status?, timestamp_ms, data, event_id?, field?, event?, group_id?, participant_id? }`: verified Cloud webhook events after persistence |

Delivery is best effort and not retried: target return values do not send replies. Set `namespace` on the observed source when the worker runs outside the observer's namespace; the target resolves in the observing worker's namespace, and observer metadata reaches the target as the call's metadata.

### Native events

Native kinds include `session.connected`, `session.disconnected`, `session.logged_out`, `session.qr`, `message.received`, `message.receipt`, `history.sync`, `group.updated` and `chat.presence`. Observe one session's incoming messages:

```ts
iii.observe(
  {
    function_id: 'whatsapp::on-events',
    filters: { 'input.session_id': { $eq: 'support' }, 'input.kind': { $eq: 'message.received' } },
    deliver: 'output',
  },
  'automation::on-whatsapp-message',
)
```

Native event polling with `whatsapp::events::poll` reconciles a bounded in-memory event history. Read the [native event contract](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md#functions-and-events) before observing.

### Cloud events

Kinds are `message.received`, `message.status` and `webhook.received`; an observer without a filter on `input.kind` receives all three. Clauses combine with AND, and `$in` lists alternatives. Observe incoming text messages from one contact:

```ts
iii.observe(
  {
    function_id: 'whatsapp::cloud::on-events',
    filters: {
      'input.kind': { $eq: 'message.received' },
      'input.chat_id': { $eq: '+15550000009' },
      'input.message_type': { $eq: 'text' },
    },
    deliver: 'output',
  },
  'automation::on-whatsapp-message',
)
```

Read the [observer walkthrough](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md#filtered-cloud-webhook-events) and [complete filters and payloads](https://github.com/iii-hq/workers/blob/main/whatsapp/cloud_events.md). Cloud deduplication is persistent, but events can still be lost; reconcile through the Cloud chats and messages archive.

### What the worker observes

| Function | Args or filters | Target |
| --- | --- | --- |
| `configuration::on-updated` | filters `input.id` (the configuration entry) and `input.event_type` `configuration:updated` | `whatsapp::on-config-change` |
| `http::on-request` | args `{ method: GET \| POST, path: /webhooks/whatsapp-cloud/<configuration ID>, public_webhook: true }` | `whatsapp::cloud::webhook::verify`, `whatsapp::cloud::webhook::receive` |
| `http::on-request` | args `{ method: POST, path: /webhooks/whatsapp-flows/<configuration ID>, public_webhook: true }` | `whatsapp::cloud::flows::endpoint` |
| `console::ui::asset` | args `{ kind: script, path: whatsapp/page.js }`, `{ kind: style, path: whatsapp/styles.css }` | `whatsapp::ui-content` |
| `quick-tunnel::on-changed` | filter `input.tunnel_id` `webhooks` | `whatsapp::cloud::tunnel::changed` |

Each observer stays pending until its provider runs; a refused observer is logged and the worker keeps serving its own functions.

### Functions nobody can observe

These functions carry a credential in their input or output. They are registered with `observable: false`, so observing them fails with `NOT_OBSERVABLE`; calling them works as for any other function. Every other function the worker registers is observable.

| Function | Credential |
| --- | --- |
| `whatsapp::sessions::qr` | output: the pairing QR payload |
| `whatsapp::sessions::pair_code` | output: the phone pairing code |
| `whatsapp::cloud::phone_numbers::verify_code` | input: the verification code Meta sent to the phone |
| `whatsapp::cloud::phone_numbers::register` | input: the two-step verification PIN |
| `whatsapp::cloud::phone_numbers::set_pin` | input: the two-step verification PIN |
| `whatsapp::cloud::webhook::verify` | input: Meta's `hub.verify_token`, which equals the configured verify token |
| `whatsapp::on-config-change` | input: the `configuration::on-updated` event, whose `new_value` holds the access token, app secret, verify token and Flow private key |

## Limits and differences from OpenWA

Use this worker's typed functions and inspect their contracts with `iii worker info whatsapp`. It is not an HTTP-compatible OpenWA deployment. The [native API boundaries](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md#limits-and-differences-from-openwa) list unsupported operations and nullable metadata.

Archives contain captured messages, not a complete remote history. Media transfers use bounded bytes; arbitrary host paths and download URLs are not accepted. The worker does not transcode files, host call audio, or settle payments. Native protocol support depends on the unofficial whatsmeow client; Cloud support depends on Meta's account permissions and feature availability.

## Migration notes

Rust credentials are incompatible with whatsmeow. Use a fresh `data/whatsapp-go` directory and explicitly pair a new session; keep the old data for the Rust worker. Existing Go SQLite stores require an explicit migration to the database worker. Follow the [storage migration guide](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/database-storage.md#migrate-existing-go-storage) before upgrading an existing account.

## Further documentation

Find task-specific details in these guides:

- [ADE setup, test results, archives, and draft behavior](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/ade.md)
- [Cloud credentials, webhooks, documents, and event observers](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md)
- [Linked-phone pairing, functions, and events](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md)
- [Database storage and migration](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/database-storage.md)
- [Build from source, offline validation, and local SDK development](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/development.md)

## License

Apache-2.0. See the [repository license](https://github.com/iii-hq/workers/blob/main/LICENSE).
