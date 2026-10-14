# Cloud Flows

Flows use the existing configured WABA, phone number, credentials, and fixed
Graph API origin. All functions below have the prefix `whatsapp::cloud::flows::`.
Root Flow IDs are checked against the configured WABA before use. Meta decides
whether a draft can be published, edited, deleted, or deprecated.

| Suffix | Request |
| --- | --- |
| `create` | `name`, `categories`; optional `clone_flow_id`, `endpoint_uri`, `flow_json`, `publish` |
| `list` | Optional `limit` (1–100, default 50), `after` |
| `get` | `flow_id` |
| `update` | `flow_id`, `name`; optional `categories`, `endpoint_uri` |
| `delete` | `flow_id`; only draft Flows can be deleted |
| `publish` | `flow_id`; publishing is irreversible |
| `deprecate` | `flow_id`; deprecation is irreversible |
| `preview` | `flow_id`; returns a preview without invalidating existing previews |
| `assets::list` | `flow_id`; returns names/types and `next_cursor` |
| `assets::upload` | `flow_id`, `data_base64`; exact JSON bytes, fixed `flow.json` filename |
| `assets::download` | `flow_id`; returns `data_base64`, `file_length`, `mimetype`, `name`, `asset_type` |
| `encryption::get` | `{}`; reads the configured phone public key and signature status |
| `encryption::set` | Optional `public_key`; otherwise derives the public key from the configured private key |
| `endpoint::status` | `{}`; local callback path and readiness, without credentials |

Allowed categories are `SIGN_UP`, `SIGN_IN`, `APPOINTMENT_BOOKING`,
`LEAD_GENERATION`, `CONTACT_US`, `CUSTOMER_SUPPORT`, `SURVEY`, and `OTHER`.
JSON assets are limited to `min(max_media_bytes, 10000000)` decoded bytes.
Uploads use multipart `file`, `name=flow.json`, and `asset_type=FLOW_JSON`.
Downloads resolve the asset through Meta, validate the HTTPS origin and public
DNS address, enforce MIME/size limits, and never follow redirects or retry.
The caller cannot supply a download URL or host path.

Create a draft:

```json
{
  "name": "Report request",
  "categories": ["CUSTOMER_SUPPORT"],
  "endpoint_uri": "https://your-public-host.example/webhooks/whatsapp-flows/my-project-whatsapp-go"
}
```

The response includes its `id`. Use that ID as `flow_id` for later operations.
Upload a valid Flow JSON document through `assets::upload`; inspect `success`
and `validation_errors` before explicitly invoking `publish`. A successful
HTTP request can still contain Flow JSON validation errors. The worker preserves
these diagnostics with credential redaction.

To send the Flow, use `whatsapp::cloud::messages::send_flow`; see
[interactive message contracts](cloud-interactive.md). Existing approved Flow
templates can also use `whatsapp::cloud::messages::send_template`.

## Encrypted data exchange endpoint

The worker registers `POST /webhooks/whatsapp-flows/<configuration-id>`.
It is inactive until encryption and the business handler are configured.
These optional configuration fields also accept runtime-only environment
overrides:

| Configuration field | Environment override |
| --- | --- |
| `cloud.flow_private_key` | `WHATSAPP_FLOW_PRIVATE_KEY` |
| `cloud.flow_handler_function` | `WHATSAPP_FLOW_HANDLER_FUNCTION` |
| `cloud.flow_handler_namespace` | `WHATSAPP_FLOW_HANDLER_NAMESPACE` |

Supply an unencrypted PEM RSA-2048 private key as the environment value, using
actual newlines. No host key-file path or private-key result is exposed. Publish
only its public key with `encryption::set`. The existing Meta app secret verifies
`X-Hub-Signature-256` before any decryption or business handler invocation.

Requests use RSA-OAEP/SHA-256 and AES-128-GCM, including Meta's inverted response
IV. Invalid authentication returns 403; decryption failure returns 421. Bodies
are limited to 256 KiB, and decrypted request/handler response JSON to 64 KiB.
Only data API version `3.0` is accepted. `ping` is answered locally. Signed,
decrypted `INIT`/`data_exchange` error notifications (`data.error`) receive the
required encrypted `{"data":{"acknowledged":true}}` response locally; they do
not invoke the business handler again.

For `INIT`, `BACK`, or `data_exchange`, the configured iii function receives the
decrypted object. If its namespace is empty, the worker namespace is used.
Private keys and configured Meta credentials are not passed to that handler.
The WhatsApp process disables SDK trace payload capture, but the handler process
controls its own telemetry and logging. Configure `III_DISABLE_TRACE_PAYLOADS=1`
on that process too if Flow session tokens and customer data must stay out of
SDK traces, and avoid logging these payloads in application code. Endpoint
`trace_hidden` metadata does not enforce the handler's telemetry policy.
For example, handler `reports::flow` receives:

```json
{
  "version": "3.0",
  "action": "INIT",
  "flow_token": "a-caller-generated-unpredictable-session-token"
}
```

The handler returns a screen defined in the published Flow and its data:

```json
{
  "screen": "REPORT",
  "data": {
    "available_formats": [
      {"id": "pdf", "title": "PDF"},
      {"id": "txt", "title": "Plain text"}
    ]
  }
}
```

The worker encrypts that response for Meta. The handler owns authorization of
the `flow_token`, business rules, and idempotency of side effects; the worker
does not invent the contents of an application's form. Handler dispatch has an
eight-second timeout and no automatic retry. Meta may make another request, so
the application must not assume that one user interaction invokes its handler
exactly once across network failures.

Point the Flow's `endpoint_uri` at this route on an existing public HTTPS origin.
The development Quick Tunnel supplies an origin; replace the standard message
webhook path with the path returned by `endpoint::status`. Tunnel URLs are
temporary and are never automatically written into Meta configuration.

Sources checked October 2, 2026:
[Flows API](https://developers.facebook.com/documentation/business-messaging/whatsapp/flows/guides/flowsapi),
[encrypted endpoints](https://developers.facebook.com/documentation/business-messaging/whatsapp/flows/guides/implementingyourflowendpoint),
[phone encryption keys](https://developers.facebook.com/documentation/business-messaging/whatsapp/flows/guides/whatsapp-business-encryption).
