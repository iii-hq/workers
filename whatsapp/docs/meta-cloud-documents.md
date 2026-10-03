# Cloud API document contract and Meta references

Checked against Meta's official documentation on October 2, 2026. This note
distinguishes Meta's documented behavior from this worker's validation policy.

## Meta endpoints and lifecycle

Upload to `POST https://graph.facebook.com/{version}/{phone_number_id}/media`
with bearer authentication and multipart form data. Include
`messaging_product=whatsapp`, the `file` bytes, and their MIME type. Meta's
parameter table names `type`; its example sets the file part's `Content-Type`.
The worker supplies both. Success returns an `id` for later sends. Uploads
persist for 30 days unless deleted earlier; IDs received through inbound
webhooks expire after seven days. Download URLs have a separate five-minute
lifetime. These are not interchangeable expiration periods.
See [Meta: Media](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media)
and [Media Upload API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/media-upload-api).

The existing configured token supplies `whatsapp_business_messaging`
permission. Calls use the configured business phone number and Graph API
version. Credentials belong in worker configuration, never the upload/send
payload. See [Meta: Media authentication](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media#upload-media).

Send to `POST https://graph.facebook.com/{version}/{phone_number_id}/messages`
using this JSON shape for a phone recipient:

```json
{
  "messaging_product": "whatsapp",
  "recipient_type": "individual",
  "to": "+15550000001",
  "type": "document",
  "document": {
    "id": "1376223850470843",
    "filename": "report.txt",
    "caption": "Your requested report"
  }
}
```

`caption` is optional and limited to 1,024 characters. Meta also makes
`filename` optional; when provided it includes the extension. No numeric
filename-length limit is published on that page. See
[Meta: Document messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/document-messages).

## Document allowlist and Markdown

Meta lists a maximum of **100 MB** for each document type below. Its pages do
not define that unit in bytes. The worker uses a conservative ceiling of
100,000,000 bytes, further limited by `max_media_bytes`; this is an explicit
worker policy, not a claim that Meta documents that exact byte count.

| Extension | MIME type |
| --- | --- |
| `.txt` | `text/plain` |
| `.pdf` | `application/pdf` |
| `.xls` | `application/vnd.ms-excel` |
| `.xlsx` | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` |
| `.doc` | `application/msword` |
| `.docx` | `application/vnd.openxmlformats-officedocument.wordprocessingml.document` |
| `.ppt` | `application/vnd.ms-powerpoint` |
| `.pptx` | `application/vnd.openxmlformats-officedocument.presentationml.presentation` |

Meta only guarantees those listed formats; its documentation says other
types may pass through the API with unsupported client behavior. Markdown
(`.md`, `text/markdown`) is absent, so this worker rejects it. The caller must
explicitly choose and supply a supported `.txt` or PDF document. There is no
automatic rename, MIME substitution, or conversion. See
[Meta: Supported document types](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/document-messages#supported-document-types).

## Service window and message outcomes

A user's incoming message or call opens or refreshes the 24-hour customer
service window. Free-form documents use that window; outside it, choose an
approved template. A local archive cannot prove the current window state,
especially when calls or earlier history are absent. Meta remains the
authority. Its send response acknowledges API acceptance, not delivery;
delivery and failure arrive through `messages` webhooks. See
[Meta: Service messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/send-messages).

Meta code `131047` indicates the closed-window restriction and directs the
caller to use a template. Media upload failures can use `131053`. Errors may
arrive synchronously, asynchronously, or both; use numeric codes and monitor
webhook failures too. The worker never switches message type automatically.
See [Meta: Error codes](https://developers.facebook.com/documentation/business-messaging/whatsapp/support/error-codes).

No retry is performed for upload or send. A timeout or disconnected response
can leave the outcome unknown: an upload may exist without its ID reaching
the caller, or a message may already have been accepted. Do not infer failure
or automatically repeat a send. This conservative timeout behavior is worker
policy. A successful message ID can be reconciled with
[Meta: Status webhooks](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/reference/messages/status).

## Worker-specific boundaries

Upload accepts caller-supplied base64 bytes, following the existing worker
media-transfer convention. It accepts neither host filesystem paths nor
remote URLs. The existing iii engine and invoking agent's function permission
policy still govern access; registration does not grant automatic approval to
upload a file or send a message.

The effective decoded-byte limit is `min(max_media_bytes, 100000000)`. The
unchanged default is 16 MiB (16,777,216 bytes); configuration currently permits
at most 64 MiB (67,108,864 bytes). Base64 and JSON transport overhead are
additional, and the engine or caller may impose a smaller payload limit.

The worker requires a filename with a supported extension, with no directory
components or control characters, and limits it to 255 UTF-8 bytes. Its caption
validation counts Unicode code points up to 1,024. MIME declarations must
match the supported extension. The worker also performs lightweight content
checks: text is valid UTF-8 without NUL bytes; PDFs start with `%PDF-`; legacy
Office files have an OLE header; and OOXML files have ZIP entries for
`[Content_Types].xml` and their expected document, workbook, or presentation
part. ZIP contents are not decompressed. These checks are not a complete file
validator or malware scanner; Meta's validation still applies.

`media_id` is a string containing 1..128 digits. The 128-character bound is
worker policy, not a published Meta limit. The sender accepts existing IDs;
Meta validates their availability and the configured account's access.

The functions, complete request/response examples, and distinction between
Cloud and linked-device transport are documented in the
[worker README](../README.md#upload-and-send-documents).

## Structured worker errors

Failures use the existing iii invocation error body with `code` and `message`.
There is no additional structured `details` object. Callers should branch on
`code`, not parse English messages. Safe HTTP status, numeric Meta code, and
numeric subcode can appear in the message; Meta's free-form error text and
details are discarded because they can echo credentials. For example:

```json
{
  "code": "cloud_api_error",
  "message": "Cloud API HTTP 400 (code 100)"
}
```

| Code | Meaning and caller action |
| --- | --- |
| `invalid_request` | The schema, recipient, filename, caption, or base64 is invalid. Correct the input before a new call. |
| `cloud_unsupported_document` | The MIME type, extension, or content is unsupported or mismatched. Markdown is rejected here; request an explicit supported-format choice. |
| `cloud_media_too_large` | Decoded content exceeds the worker's effective byte limit. Choose a smaller file or review the configured limit. |
| `cloud_disabled` | Cloud API is disabled in the effective worker configuration. |
| `cloud_not_configured` | Required Cloud credentials, IDs, or configuration are missing or invalid. |
| `cloud_request_canceled` | Cancellation was observed before dispatch; no HTTP call was made. |
| `cloud_api_error` | Meta returned an unsuccessful HTTP response or Graph error. Numeric diagnostics are retained without remote error text. |
| `cloud_window_closed` | Meta returned `131047`. Wait for a new user message/call, or explicitly choose an approved template. No automatic fallback occurs. |
| `cloud_outcome_unknown` | A dispatched POST timed out, failed in transport, returned an unreadable/invalid acknowledgement, or received a 5xx response without a valid Meta error code. Meta may have completed it. Do not retry automatically. |
| `cloud_storage_error` | The local archive could not be prepared before sending. No message POST was made. |
| `cloud_archive_failed` | Meta accepted the message, but local archival failed. The error message includes its message ID. Reconcile with webhook receipts; do not resend. |

These errors never initiate a retry. An invoking client can also time out
before receiving the worker's result; that client-side timeout likewise does
not establish that a send failed.

## Local validation

`cloud_documents_test.go` exercises the HTTP boundary with local mock servers:
multipart fields and original bytes, quoted UTF-8 filenames, reusable media
IDs, MIME and size rejection, missing credentials, Meta errors, closed-window
responses, ambiguous timeouts/gateway failures, redirects, and no retries.
It also verifies that acceptance is archived as submitted until a delivery
webhook arrives. `cloud_test.go` covers the existing text/template JSON
transport; the full Go suite covers native behavior and webhook processing.

The optional engine smoke test discovers both document schemas from an
isolated engine and checks the structured Markdown rejection through the SDK.
Run the [README validation commands](../README.md#validation) with temporary
engine/worker binaries. These checks do not pair an account, call Meta, send a
real message, or update the live worker.
