# Cloud API media

The official Cloud API transport supports separate media upload and message send operations. Upload once, then reuse the returned `media_id` in multiple sends. These functions do not use linked-device sessions, native JIDs, local file paths or caller-provided URLs.

## Functions

All IDs below use the `whatsapp::cloud::` prefix and the configured business account and phone number.

| Function suffix | Input | Output |
| --- | --- | --- |
| `media::upload` | `media_type`, `data_base64`, `mimetype`, `file_name` | `media_id`, `media_type`, `mimetype`, `file_name`, `file_length` |
| `media::upload_voicemail_announcement` | `data_base64`, `mimetype`, `file_name`, optional `description` | Upload fields plus `use_case` |
| `media::get` | `media_id` | `media_id`, `mimetype`, `file_length`, `sha256` |
| `media::download` | `media_id` | Metadata plus `data_base64` |
| `media::delete` | `media_id` | `media_id`, `deleted` |
| `messages::send_image` | `chat_id`, `media_id`, optional `caption` | `CloudSentMessage` |
| `messages::send_audio` | `chat_id`, `media_id` | `CloudSentMessage` |
| `messages::send_video` | `chat_id`, `media_id`, optional `caption` | `CloudSentMessage` |
| `messages::send_sticker` | `chat_id`, `media_id` | `CloudSentMessage` |

The four new send functions also accept optional `reply_to_message_id` and `recipient_identity_key_hash`. Captions are at most 1024 Unicode characters and are rejected for audio and stickers. `chat_id` accepts an international phone number or business-scoped user ID. `media_id` must be the ID of a suitable asset uploaded to the same Cloud account. The existing `media::upload_document` and `messages::send_document` functions remain available; document upload through `media::upload` delegates to the same implementation.

## Supported media and limits

The worker enforces the smaller of configured `max_media_bytes` and the relevant Meta limit, interpreted conservatively as decimal bytes. Static and animated stickers have separate limits detected from their WebP container. The worker checks MIME, filename extension and content headers; it does not transcode. Meta remains authoritative for full codec compatibility and account eligibility. [Official supported media documentation](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media#supported-media-types)

| `media_type` | MIME types | Extensions | Meta limit |
| --- | --- | --- | --- |
| `image` | `image/jpeg`, `image/png` | `.jpg`/`.jpeg`, `.png` | 5 MB |
| `audio` | `audio/aac`, `audio/amr`, `audio/mpeg`, `audio/mp4`, `audio/ogg` | `.aac`, `.amr`, `.mp3`, `.m4a`, `.ogg` | 16 MB |
| `video` | `video/mp4`, `video/3gpp` | `.mp4`, `.3gp` | 16 MB |
| `sticker` | `image/webp` | `.webp` | 100 KB static; 500 KB animated |
| `document` | Supported text, PDF and Office MIME types | `.txt`, `.pdf`, `.doc`, `.docx`, `.xls`, `.xlsx`, `.ppt`, `.pptx` | 100 MB |

Images must be 8-bit RGB/RGBA. Ogg audio must contain mono Opus; `audio/ogg; codecs=opus` is also accepted and normalized to `audio/ogg`. MP4/3GPP video must use H.264 with at most one AAC audio stream. Container metadata is inspected without installing codec libraries or spawning processes. See [document-specific restrictions](meta-cloud-documents.md), including explicit rejection of Markdown: choose and provide a supported `.txt` or PDF yourself; no silent renaming or conversion occurs.

The default worker limit is 16 MiB and configuration currently allows at most 64 MiB. Consequently, some files permitted by Meta's 100 MB document ceiling remain too large for this worker.

## Complete upload and send example

Invoke `whatsapp::cloud::media::upload` with this valid one-pixel RGBA PNG:

```json
{
  "media_type": "image",
  "data_base64": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQiTL6DwACxgGg6e3a2QAAAABJRU5ErkJggg==",
  "mimetype": "image/png",
  "file_name": "example.png"
}
```

An illustrative acknowledgement is:

```json
{
  "media_id": "987654321",
  "media_type": "image",
  "mimetype": "image/png",
  "file_name": "example.png",
  "file_length": 70
}
```

Use the **actual returned** `media_id` and an authorized recipient with an open customer-service window when invoking `whatsapp::cloud::messages::send_image`:

```json
{
  "chat_id": "+15551234567",
  "media_id": "987654321",
  "caption": "Example image",
  "reply_to_message_id": "wamid.original-message"
}
```

Omit `reply_to_message_id` for a new message. A send response has the existing format:

```json
{
  "message_id": "wamid.accepted-message",
  "chat_id": "+15551234567",
  "status": "accepted",
  "message_status": "accepted"
}
```

`message_status` can be `null`. API acceptance does not confirm delivery: the local archive initially records `submitted`; the configured status webhook confirms `sent`, `delivered`, `read` or `failed`. If the customer-service window is closed, the worker returns `cloud_window_closed`; it does not silently substitute a template. [Messaging rules](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/send-messages)

## Received attachments, metadata and deletion

An incoming media webhook exposes the media ID in the corresponding raw event payload. Pass that ID to `whatsapp::cloud::media::get`, `whatsapp::cloud::media::download` or `whatsapp::cloud::media::delete`:

```json
{"media_id":"987654321"}
```

`get` returns safe metadata only. `download` retrieves a fresh URL internally, then returns the verified bytes as base64. Neither operation exposes the temporary authenticated URL. SHA-256 is returned as lowercase hexadecimal. Media queries and deletion include the configured `phone_number_id`, allowing Meta to enforce ownership. Deletion returns `{"media_id":"987654321","deleted":true}` only after Meta confirms success.

Uploaded media IDs expire after 30 days; IDs from incoming webhooks expire after 7 days. Temporary download URLs expire after 5 minutes. The worker makes no background renewal attempt. [Media lifecycle and endpoints](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media)

## Voicemail announcements

`whatsapp::cloud::media::upload_voicemail_announcement` is separate from ordinary messaging uploads. It requires mono Opus Ogg shorter than 60 seconds, validates duration from the complete Ogg stream's final granule position, and uploads with Meta's required `use_case=call_voicemail_announcement`. Its optional `description` is limited to 1024 characters by worker policy. The acknowledgement adds `"use_case":"call_voicemail_announcement"`.

Use this returned ID in `whatsapp::cloud::calls::settings::update` under `calling.voicemail.audio.default.announcement_media_id`. Such assets are exempt from the normal messaging-media TTL and **cannot be sent as regular messages**. No conversion, transcoding or duration trimming is performed. [Official voicemail requirements](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/call-settings#voicemail)

## Security and failure behavior

Downloads accept only an ID. The worker obtains the URL from authenticated Meta metadata, requires HTTPS and an allowed Meta media hostname, rejects private/reserved DNS results, connects to the checked IP, disables proxy bypass and redirects, enforces byte/MIME checks and verifies the returned size and SHA-256. Supported download hosts are `lookaside.fbsbx.com`, `mmg.whatsapp.net` and subdomains of `fbcdn.net`; another future Meta CDN host produces `cloud_unsafe_media_url` until reviewed and added. [Authenticated downloads](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/media/media-download-api)

Malformed or unsupported content fails before upload. Structured errors include `invalid_request`, `cloud_not_configured`, `cloud_unsupported_media`, `cloud_unsupported_document`, `cloud_media_too_large`, `cloud_unsafe_media_url`, `cloud_download_error`, `cloud_invalid_response` and the existing safe Meta errors. Credentials, remote diagnostic text and download URLs are not returned in errors. No operation automatically retries. A mutating request whose acknowledgement is lost returns `cloud_outcome_unknown`; Meta may have completed it, so inspect state before explicitly choosing to retry.
