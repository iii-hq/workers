# Official Cloud groups and calling

These functions use the configured Meta Cloud phone and its existing access token. They are separate from the worker's linked-device `whatsapp::groups::*` and `whatsapp::calls::*` operations. They do not enable features or contact recipients until explicitly invoked.

## Groups eligibility and limits

Meta Groups API requires an Official Business Account. It allows up to 8 participants, 10,000 groups per business phone and one Cloud business per group. Groups are not available for Business App coexistence numbers or Multi-solution Conversations. Supported messages are text, media and eligible text/media templates; calling, authentication, commerce and interactive messages are not supported in groups. The worker does not bypass those eligibility checks. [Groups API](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups)

Groups are invite-only: an application cannot add an arbitrary user directly. Users join through a link and, when enabled, approval. Removing a participant prevents that participant from joining again through the group's invite link. Resetting an invite link invalidates previous links. [Group management](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/reference)

All group-specific reads, mutations and sends verify that the group appears under the configured phone's `/groups` edge. Enumeration is bounded and uses opaque cursors. Root Graph IDs are escaped as path segments. A broadly authorized Meta token therefore does not let a caller select another phone's group through these functions.

## Group functions

All function IDs in this table have prefix `whatsapp::cloud::groups::`.

| Suffix | Required input | Optional input |
| --- | --- | --- |
| `list` | None | `limit` (1–1024), `after` |
| `get` | `group_id` | `fields` from the schema's allowlist |
| `create` | `subject` | `description`, `join_approval_mode` |
| `update` | `group_id`, at least one setting | `subject`, `description` |
| `delete` | `group_id` | None |
| `photo::set` | `group_id`, `data_base64`, `mimetype`, `file_name` | None |
| `invite_link::get` | `group_id` | None |
| `invite_link::reset` | `group_id` | None |
| `join_requests::list` | `group_id` | `limit` (1–100), `after` |
| `join_requests::approve` | `group_id`, `join_request_ids` | None |
| `join_requests::reject` | `group_id`, `join_request_ids` | None |
| `participants::remove` | `group_id`, `participant_ids` (1–8) | None |
| `messages::send_text` | `group_id`, `text` | `preview_url`, `reply_to_message_id` |
| `messages::send_media` | `group_id`, `media_type`, `media_id` | `caption`, document `file_name`, `reply_to_message_id` |
| `messages::send_template` | `group_id`, `name`, `language` | `components`, `reply_to_message_id` |
| `messages::pin` | `group_id`, `message_id`, `expiration_days` (1–30) | None |
| `messages::unpin` | `group_id`, `message_id` | None |

Group IDs are opaque Meta identifiers, not phone numbers or native JIDs. The `group_id` request field contains the raw ID. Cloud conversation APIs and the local archive identify the same conversation as `group:<group_id>`; storage internally adds `cloud:`. Send results contain that logical conversation ID in `chat_id` and `status: "accepted"`.

Management results retain Meta's response shape, with credential fields and authenticated pagination URLs removed. Inspect per-request failures when approving/rejecting requests or removing participants. Creation, deletion and settings changes complete asynchronously; API acknowledgement is not a group lifecycle confirmation. [Group webhook contracts](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/webhooks)

### Create and invite

Invoke `whatsapp::cloud::groups::create`:

```json
{
  "subject": "Customer support",
  "description": "Coordinate the customer's support request.",
  "join_approval_mode": "approval_required"
}
```

Subscribe the Meta app to `group_lifecycle_update`, `group_participants_update`, `group_settings_update` and `group_status_update`. A successful `group_lifecycle_update` contains `value.groups[]` with `group_id`, `type: "group_create"`, `request_id` and `invite_link`. Use the actual returned group ID in subsequent calls. Approval requests use the `join_request_id` obtained from the list or webhook, not a phone number:

```json
{
  "group_id": "Y2FwaV9ncm91cDoxNzA1NTU1MDEzOToxMjAzNjM0MDQ2OTQyMzM4MjAZD",
  "join_request_ids": ["MTY0NjcwNDM1OTU6MTIwMzYzNDA0Njk0MjMzODIw"]
}
```

The worker limits approval/rejection batches to 100 IDs. To invite a customer outside the service window, use an approved group-invite template through the existing individual `whatsapp::cloud::messages::send_template`; its body component parameters can include `{"type":"group_id","group_id":"<actual-group-id>"}`. [Official invitation template procedure](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/reference#send-group-invite-link-template-message)

### Send to the group

Invoke `whatsapp::cloud::groups::messages::send_media` after uploading an image through [Cloud media upload](cloud-media.md):

```json
{
  "group_id": "Y2FwaV9ncm91cDoxNzA1NTU1MDEzOToxMjAzNjM0MDQ2OTQyMzM4MjAZD",
  "media_type": "image",
  "media_id": "987654321",
  "caption": "Project update"
}
```

Use actual IDs, not these illustrative values. `media_type` supports `image`, `audio`, `video`, `sticker`, `document`; `file_name` is required only for documents and captions are valid only for image/video/document. Group messages use `recipient_type: "group"` with the raw group ID in Meta's `to` field. Incoming webhooks include `messages[].group_id`, while `from` identifies the individual sender. Status events identify the group through `recipient_type: "group"` and `recipient_id`. [Group messaging](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/groups-messaging)

`photo::set` accepts only JPEG base64, constrained to `min(max_media_bytes, 5 MB)`, square dimensions and at least 192×192 pixels. It uploads multipart bytes directly; host paths and URLs are not accepted. The schema follows the ordinary media transfer conventions.

## Calling functions

All function IDs below have prefix `whatsapp::cloud::calls::`.

| Suffix | Input | Result |
| --- | --- | --- |
| `settings::get` | `{}` | Sanitized current settings; SIP passwords are never requested or returned |
| `settings::update` | Typed `calling` object | Meta update acknowledgement |
| `permissions::get` | `chat_id` | Permission state and current action limits |
| `permissions::request` | `chat_id`, optional `text`, `reply_to_message_id`, `recipient_identity_key_hash` | Normal message acceptance |
| `connect` | `chat_id`, `session`, optional `biz_opaque_callback_data` | `call_id`, `action`, `status: "accepted"` |
| `pre_accept` | `call_id`, `session`, optional `biz_opaque_callback_data` | Same call result |
| `accept` | `call_id`, optional `session`, `biz_opaque_callback_data` | Same call result |
| `reject` | `call_id` | Same call result |
| `terminate` | `call_id` | Same call result |

Calling remains subject to Meta country, account, billing and user-permission requirements. Query permission state before initiating an outbound call. A permission request is an interactive message subject to the customer-service window; it does not grant permission automatically. [Calling overview](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling), [user permissions](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/user-call-permissions)

The worker exposes **signaling and settings**, not a WebRTC implementation, microphone, audio relay, SIP server or browser calling UI. The caller must provide a valid SDP offer/answer generated by its media endpoint and handle media transport. Calling action acceptance is not proof that the peer answered or that media is flowing. Observe call webhooks. With SIP enabled, use the configured external SIP system for signaling according to Meta's rules. SIP credentials must be provisioned through a separate secure process; this worker intentionally does not disclose them. [Calling reference](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/reference), [SIP integration](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/sip)

### Request calling permission

Invoke `whatsapp::cloud::calls::permissions::request`:

```json
{
  "chat_id": "+15551234567",
  "text": "May we call you to help with your support request?"
}
```

After the user responds, invoke `whatsapp::cloud::calls::permissions::get` with `{"chat_id":"+15551234567"}`. A BSUID such as `US.13491208655302741918` can also identify the recipient. Meta returns the permission state and action limits; the worker does not infer permission from a previously sent request.

### Call settings and voicemail

The typed `calling` object supports status, icon visibility, country restrictions, callback permissions, weekly/holiday hours, one SIP server, additional PCMA/PCMU codecs, and voicemail. Call-hour updates replace existing schedules; omitting a holiday schedule when replacing call hours clears it according to Meta's contract. Overlapping intervals, invalid time zones and invalid codec/status values are rejected locally. [Settings reference](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/call-settings)

For voicemail, first use `whatsapp::cloud::media::upload_voicemail_announcement` with complete mono Opus Ogg shorter than 60 seconds. Pass the returned media ID as a **string** to preserve its digits; the worker encodes it as a JSON integer for Meta:

```json
{
  "calling": {
    "status": "ENABLED",
    "call_hours": {"status": "DISABLED"},
    "voicemail": {
      "status": "ENABLED",
      "triggers": ["TIMEOUT", "REJECT"],
      "audio": {
        "default": {
          "announcement_media_id": "987654321",
          "timeout_seconds": 20
        }
      }
    }
  }
}
```

The announcement media must use the special voicemail upload use case. Ordinary messaging-media IDs do not satisfy that requirement. The worker validates the request; Meta validates the asset's use case and eligibility. Voicemail arrives as incoming audio messages through the webhook. [Voicemail requirements](https://developers.facebook.com/documentation/business-messaging/whatsapp/calling/call-settings#voicemail)

## Errors and retries

All functions reuse the existing Cloud credentials, configured API version, bounded HTTP responses and no-retry policy. They never switch to the native transport. Unowned groups return `cloud_group_not_found`; invalid local fields return `invalid_request`; safe numeric Meta errors retain `cloud_api_error` or the existing specific error code. Unreadable mutation acknowledgements and interrupted calls can return `cloud_outcome_unknown`. Check remote state before deliberately trying again.
