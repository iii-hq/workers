# Configure Meta Cloud API and webhooks

Set up an existing WhatsApp Business Platform account, expose its signed webhook, and register filtered callbacks. For first use, follow the [consumer README](../README.md).

## Table of contents

- [Account setup](#account-setup)
- [Worker environment configuration](#worker-environment-configuration)
- [Central environment references and local files](#central-environment-references-and-local-files)
- [Development webhook URL](#development-webhook-url)
- [Upload and send documents](#upload-and-send-documents)
- [Cloud message archive](#cloud-message-archive)
- [Filtered Cloud webhook events](#filtered-cloud-webhook-events)

## Account setup

The **Meta Cloud API** connection method works without a linked-device session. It supports an
existing WhatsApp Business Platform account with local readiness, an explicit
read-only connection check, message composition, and a webhook-backed archive.
Typed functions additionally expose media transfer, interactive messages,
template/account management, Flows, and eligible group/calling/commerce APIs.
Operations that change Meta settings run only when explicitly invoked; enabling
the worker does not automatically register a number or subscribe an app.

Detailed contracts and examples:

- [Coverage, remaining boundaries, and contract index](cloud-api.md)
- [Media upload, download, and sends](cloud-media.md)
- [Documents](meta-cloud-documents.md)
- [Read receipts, reactions, and interactive messages](cloud-interactive.md)
- [Template, profile, phone, and account management](cloud-management.md)
- [Flows lifecycle and encrypted data exchange](cloud-flows.md)
- [Groups and calling](cloud-groups-calls.md)
- [Commerce, payments, and marketing](cloud-commerce.md)
- [Webhook trigger filters and callback payloads](../cloud_events.md)

In ADE, open **Configure worker** and find **Meta Cloud API**. Enter the existing
WhatsApp Business Account ID and Phone Number ID, then the access token, Meta
App Secret, and a verify token of your choice. Enable Cloud API and save.
Saved Cloud settings apply live unless overridden by the worker environment.
Secret inputs are masked; worker status and debug output omit their values.

## Worker environment configuration

Set the following variables in the WhatsApp worker's environment. They work
with both central ADE configuration and `--local-config`:

| Environment variable | Configuration field | Accepted value |
| --- | --- | --- |
| `WHATSAPP_CLOUD_ENABLED` | `cloud.enabled` | Exactly `true` or `false` |
| `WHATSAPP_CLOUD_API_VERSION` | `cloud.api_version` | A Graph API version, such as `v26.0` |
| `WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID` | `cloud.business_account_id` | Numeric WABA ID |
| `WHATSAPP_CLOUD_PHONE_NUMBER_ID` | `cloud.phone_number_id` | Numeric Meta phone number ID |
| `WHATSAPP_CLOUD_ACCESS_TOKEN` | `cloud.access_token` | Token authorized for the WABA |
| `WHATSAPP_META_APP_SECRET` | `cloud.app_secret` | Meta app secret for webhook signatures |
| `WHATSAPP_WEBHOOK_VERIFY_TOKEN` | `cloud.verify_token` | Token you also enter in Meta's webhook settings |

For iii Compose, add an uncommitted environment file to the WhatsApp container:

```yaml
containers:
  whatsapp:
    worker: path://../whatsapp
    env_file:
      - ./whatsapp.env
```

Example `whatsapp.env` (replace the example IDs and credentials):

```dotenv
WHATSAPP_CLOUD_ENABLED=true
WHATSAPP_CLOUD_API_VERSION=v26.0
WHATSAPP_CLOUD_BUSINESS_ACCOUNT_ID=123456789012345
WHATSAPP_CLOUD_PHONE_NUMBER_ID=987654321098765
WHATSAPP_CLOUD_ACCESS_TOKEN=your_access_token_here
WHATSAPP_META_APP_SECRET=your_app_secret_here
WHATSAPP_WEBHOOK_VERIFY_TOKEN=your_webhook_verify_token_here
```

Keep the file outside version control and restrict access, for example with
`chmod 600 whatsapp.env`. Compose's `environment` mapping or exported process
variables can supply the same names. Restart the WhatsApp worker after changing
its deployment environment.

A defined variable overrides its corresponding saved or local field. An unset
variable preserves that field's configured value or default. An empty value is
an explicit override, not a fallback; normal validation still applies. For
example, an empty enable flag is invalid, and an enabled Cloud API needs IDs
and an access token. Values are used as supplied, without recursive `${...}`
expansion; numeric IDs remain exact strings.

Environment values are applied only at runtime and are never written back by
the worker to central configuration or its YAML seed. Existing saved secrets
are not automatically removed. ADE and `whatsapp::config-status` show which
fields are overridden and their variable names, without exposing secret values.
Saved edits to those fields take effect only after removing the corresponding
environment override and restarting the worker.

## Central environment references and local files

ADE also preserves `${VAR}` and `${VAR:default}` references in stored
configuration. These are expanded by the configuration service using the
**engine process environment**, not the WhatsApp container's `env_file`.
Standalone references can be coerced into YAML scalar types: a numeric ID may
become a number and fail the required string schema. Prefer the direct worker
variables above for Cloud IDs and credentials.

When moving variables from the engine environment to the worker environment,
remove unused central references: central configuration must load successfully
before worker overrides can apply. Local YAML files continue to accept literal
configuration values; `${...}` interpolation is not performed on local files.
Protect any local files containing credentials.

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
with package 0.22.1). The HTTP worker's restricted webhook listener is disabled
by default. Enable it in that worker's existing configuration before exposing
the callback:

```yaml
webhook_listener:
  host: 127.0.0.1
  port: 3112
```

Keep the remaining HTTP settings intact. HTTP applies listener changes live;
confirm it opened successfully before starting a tunnel. See the
[restricted listener guide](https://github.com/iii-hq/workers/blob/main/http/README.md#restricted-webhook-listener).
Forward a public HTTPS callback to this listener, not the normal HTTP/admin
listener on port 3111. Pointing a tunnel at port 3112 does not enable it.
The worker registers GET and POST at:

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

## Development webhook URL

In **Cloud API**, select **Start development tunnel** to obtain a complete
HTTPS callback URL, then **Copy webhook URL** and paste it into the Meta app's
webhook configuration. Use the same verify token saved in **Configure worker**,
or supplied through `WHATSAPP_WEBHOOK_VERIFY_TOKEN` when that override is set.
Enable the `messages` field and subscribe the app to the WABA. Starting a
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
for other components. Recipient validation is field-linked and runs on blur,
and known required template components are checked before Send is enabled.
The advanced editor includes a read-only example derived from the selected
template and a Meta payload reference; examples are never inserted or sent
automatically. After successful template acceptance, the template choice is
remembered for this Cloud account in page memory while parameter values are
cleared, so a repeat test starts with the same template without a phantom draft.
The send footer keeps the next recipient and selected template visible while
editing; uncertain send feedback separately names the original attempted recipient.
Destinations use international phone numbers with a
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

## Upload and send documents

Documents use two separate Cloud functions. Upload once, save the returned
`media_id`, and reuse it for subsequent explicit sends while it remains valid.
Neither function needs a linked-device session or invokes native media sending.
These functions are available after building and starting this worker version;
editing source does not update an already running process.

| Function ID | Required request fields | Optional request fields |
| --- | --- | --- |
| `whatsapp::cloud::media::upload_document` | `data_base64`, `mimetype`, `file_name` | None |
| `whatsapp::cloud::messages::send_document` | `chat_id`, `media_id`, `file_name` | `caption` |

Upload uses standard base64 containing only the selected file's bytes. It
accepts no host path, URL, token, or native session ID. The calling engine or
agent must authorize each upload/send under its existing function permissions.
Do not add these functions to an automatic approval list just to test them.

Markdown `.md` / `text/markdown` is **not an officially supported Meta document
format** and is rejected. Choose `.txt` or PDF explicitly and supply that
document; the worker never renames or converts it. Meta's supported formats,
100 MB published limit, media lifetimes, and the worker's stricter limits are
listed with sources in [the document reference](meta-cloud-documents.md).

The following example assumes the user explicitly selected a plain-text
attachment named `report.txt` whose bytes are `Report` followed by a newline.
Use your actual worker namespace. The first command uploads only; the second
sends a message and should only run after confirming its recipient and file.

```sh
iii trigger whatsapp::cloud::media::upload_document --namespace my-project \
  --json '{"data_base64":"UmVwb3J0Cg==","mimetype":"text/plain","file_name":"report.txt"}'
```

Example upload result:

```json
{
  "media_id": "1376223850470843",
  "mimetype": "text/plain",
  "file_name": "report.txt",
  "file_length": 7
}
```

Replace the example ID below with the **returned** `media_id`, and replace the
example phone number with the explicitly confirmed recipient. `chat_id` uses
the same Cloud recipient rules as text messages; native JIDs are rejected.

```sh
iii trigger whatsapp::cloud::messages::send_document --namespace my-project \
  --json '{"chat_id":"+15550000001","media_id":"1376223850470843","file_name":"report.txt","caption":"Your requested report"}'
```

Example send result:

```json
{
  "message_id": "wamid.example",
  "chat_id": "+15550000001",
  "status": "accepted",
  "message_status": null
}
```

`accepted` confirms submission only. Follow the returned `message_id` in the
Cloud archive or filtered `message.status` webhook events for delivery/read
status or failure. Free-form documents require an open customer service
window; a closed-window response never triggers an automatic template send.
Upload and send each make one HTTP attempt. If the result is unknown after a
timeout, do not retry automatically or assume that no upload/message exists.

Errors preserve the iii `code` and `message` envelope. Use the stable code for
decisions: `cloud_window_closed` requires an explicit template choice or a new
customer message/call; `cloud_outcome_unknown` forbids assuming the operation
failed; `cloud_archive_failed` means Meta already accepted the message and its
ID is included in the error message. Meta's free-form error text is discarded
to avoid credential exposure. See the complete
[error contract](meta-cloud-documents.md#structured-worker-errors).

## Cloud message archive

Cloud archives use database-worker tables scoped by `<phone_number_id>`, separate
from every native session. They retain webhook messages and worker-submitted
messages under `max_inbox_messages`; changing the phone ID selects a separate
archive. They contain message previews and media labels, without media downloads
or historical backfill. Reading them sends no read receipt. Subscribe to
`whatsapp::cloud::events` for filtered webhook notifications and reconcile with
the archive; native `whatsapp::events` subscriptions remain separate.
The **Groups** tab continues to use the linked-device integration. Official
Cloud group conversations appear in the Cloud archive; their management and
send contracts are documented in [Cloud groups](cloud-groups-calls.md).

## Filtered Cloud webhook events

Read the running worker's registration guide with `engine::triggers::info`:

```json
{"id":"whatsapp::cloud::events","namespace":"messaging"}
```

Replace `messaging` with the WhatsApp worker's namespace. The returned
`description` explains registration, filters and payloads. `configuration_schema`
describes the accepted filters and `request_schema` describes each callback event.
Runtime filter validation still applies.

Trigger type `whatsapp::cloud::events` delivers one flat event per accepted
incoming message or delivery-status object after its archive transaction commits.
Administrative fields are opt-in; see the [complete event guide](../cloud_events.md).
A message event looks like:

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

The default message stream uses `message.received`; opt-in receipts use
`message.status`. Administrative webhooks use `webhook.received`, whose distinct
payload is documented in the [complete event guide](../cloud_events.md).
Every message or status event includes the account
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
| `kinds` | Event kinds; omitted or empty means only `message.received`. Set `message.status` explicitly for receipts or `webhook.received` for administrative events. |
| `chat_ids` | Canonical international phone numbers, business-scoped user IDs, or `group:<raw Cloud group ID>`. |
| `sender_ids` | Canonical sender IDs; events without a matching sender are excluded. |
| `message_types` | Exact raw Meta message types. |
| `statuses` | Exact values on `message.status` events: `sent`, `delivered`, `read` or `failed`. |
| `business_account_id` | Exact WABA ID. |
| `phone_number_id` | Exact Phone Number ID. |
| `fields` | Exact Meta webhook fields; administrative events also require `webhook.received`. |
| `events` | Exact administrative event names. |
| `group_ids` | Raw Cloud group IDs without the `group:` prefix. |
| `participant_ids` | Participants identified by group delivery/read receipts; also select `message.status`. |

List entries are alternatives (OR); populated fields combine with AND. An empty
list imposes no restriction, except for the received-only default of `kinds`.
Each list accepts at most 128 values; the complete filter is limited to 64 KiB.
Message type names use 1–64 lowercase letters, digits or underscores and start
with a letter. Account IDs are numeric strings of at most 32 digits.
Use a leading `+` for contact phone numbers, preserve business-scoped IDs such as
`BR.ENT.Abc123`, and use `group:<raw Cloud group ID>` for group archive IDs. Native JIDs are not valid Cloud filters. A status event usually
has no sender or message type, so leave those filters empty for receipt subscriptions.

### Register through an engine function

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

### Register through the Go SDK

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

Webhook deduplication is persistent in the Cloud archive's database-worker tables and
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
