# whatsapp

Send and receive WhatsApp messages through iii functions using linked phones or the official Meta WhatsApp Cloud API. Pair accounts, inspect conversations and groups, send attachments and templates, and subscribe to filtered events. An ADE page helps you configure and test each transport separately. The worker runs as a Go binary without Node.js or Chromium.

## Install

Install the worker into your existing iii Compose project:

```bash
iii trigger compose::add worker=whatsapp
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
iii trigger whatsapp::messages::send_text --json '{
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
iii trigger whatsapp::cloud::messages::send_text --json '{
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
max_inbox_messages: 10000         # Retained messages per archive
max_media_bytes: 16777216         # 16 MiB worker transfer limit
operation_timeout_secs: 30        # Deadline for subsequent operations
cloud:
  enabled: false                 # Linked phones need no Meta credentials
```

Native keys and both transports' archives persist through the database worker. Give each WhatsApp process a distinct `data_dir` and configuration identity; keep that identity and its namespace stable across upgrades. Limits and timeouts apply live; `database`, `data_dir`, and `max_events` require a restart. Invalid reloads keep the last valid settings; `whatsapp::config-status` reports the outcome and pending restart fields. Changing the database target does not migrate existing data.

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

## Custom trigger types

Bind a registered callback function to the transport you want to monitor:

| Trigger type | Fires when | Callback context |
| --- | --- | --- |
| `whatsapp::events` | Native session events, filtered by `session_id` and `kinds` | `session_id`, `sequence`, `kind`, `timestamp_ms`, and `data` |
| `whatsapp::cloud::events` | Verified Cloud webhook events after persistence, filtered by contact, group, type, status, or account | `kind`, account IDs, `timestamp_ms`, and event-specific fields |

Native event polling can reconcile a bounded in-memory event history. Read the [native event contract](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md#functions-and-events) before registering a callback.

### Filtered Cloud webhook events

Cloud subscriptions receive incoming messages by default. Delivery receipts and administrative events require explicit opt-in. Filters combine with AND; values within a list combine with OR. Read the [registration walkthrough](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md#filtered-cloud-webhook-events) and [complete filters and payloads](https://github.com/iii-hq/workers/blob/main/whatsapp/cloud_events.md).

Callbacks are best effort and are not retried automatically. Cloud deduplication is persistent, but notifications can still be lost; reconcile through the Cloud chats and messages archive. Callback return values do not send replies.

## Limits and differences from OpenWA

Use this worker's typed functions and inspect their contracts with `iii trigger engine::workers::info name=whatsapp`. It is not an HTTP-compatible OpenWA deployment. The [native API boundaries](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md#limits-and-differences-from-openwa) list unsupported operations and nullable metadata.

Archives contain captured messages, not a complete remote history. Media transfers use bounded bytes; arbitrary host paths and download URLs are not accepted. The worker does not transcode files, host call audio, or settle payments. Native protocol support depends on the unofficial whatsmeow client; Cloud support depends on Meta's account permissions and feature availability.

## Migration notes

Rust credentials are incompatible with whatsmeow. Use a fresh `data/whatsapp-go` directory and explicitly pair a new session; keep the old data for the Rust worker. Existing Go SQLite stores require an explicit migration to the database worker. Follow the [storage migration guide](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/database-storage.md#migrate-existing-go-storage) before upgrading an existing account.

## Further documentation

Find task-specific details in these guides:

- [ADE setup, test results, archives, and draft behavior](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/ade.md)
- [Cloud credentials, webhooks, documents, and trigger registration](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/cloud-setup.md)
- [Linked-phone pairing, functions, and events](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/native-api.md)
- [Database storage and migration](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/database-storage.md)
- [Build from source, offline validation, and local SDK development](https://github.com/iii-hq/workers/blob/main/whatsapp/docs/development.md)

## License

Apache-2.0. See the [repository license](https://github.com/iii-hq/workers/blob/main/LICENSE).
