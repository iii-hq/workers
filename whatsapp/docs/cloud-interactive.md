# Cloud API chat actions and interactive messages

These functions use the configured official Meta Cloud API phone number. They do not use linked-device sessions or native WhatsApp JIDs. All sends accept `chat_id` as an international number or a supported business-scoped user ID. They use the existing authenticated Cloud HTTP client and never retry automatically or fall back to native transport or templates.

The examples below are function payloads, not direct Graph requests. Invoke their function IDs in the worker's namespace. The worker maps them to `POST /{phone_number_id}/messages` with `messaging_product: "whatsapp"`.

## Delivery and failures

Message sends return the existing contract:

```json
{
  "message_id": "wamid.example",
  "chat_id": "+15551234567",
  "status": "accepted",
  "message_status": null
}
```

`accepted` means Meta accepted the request. It does not establish delivery. Subscribe to `whatsapp::cloud::events` with `kinds: ["message.status"]` for subsequent status changes. Reactions have a `sent` notification but no `delivered` or `read` notification. Incoming selections, location responses and Flow completions arrive through the existing `message.received` trigger; their original Meta data is available in the event's `data` object, subject to the existing event byte limit.

Free-form interactive, location and contact messages require an open customer service window. Meta error 131047 becomes `cloud_window_closed`; the worker does not choose or send an approved template on your behalf. A timeout or unreadable POST acknowledgement becomes `cloud_outcome_unknown`: the operation might have succeeded, and no automatic retry is performed. Inspect subsequent webhooks before choosing to resend.

All errors use the existing structured Cloud error codes, including `invalid_request`, `cloud_not_configured`, `cloud_api_error`, `cloud_window_closed`, and `cloud_outcome_unknown`. Meta error text and credential values are not copied into errors.

## Read receipts and typing

`whatsapp::cloud::messages::mark_read`:

```json
{"message_id": "wamid.incoming"}
```

`whatsapp::cloud::messages::typing` takes the same payload. It marks that incoming message as read **and** displays the text typing indicator. Meta dismisses the indicator when a reply is sent or after 25 seconds. Both functions return `{"success": true}` after Meta acknowledges the action; they create no outgoing message archive entry.

## Reactions

`whatsapp::cloud::messages::send_reaction`:

```json
{
  "chat_id": "+15551234567",
  "message_id": "wamid.incoming",
  "emoji": "👍🏽"
}
```

Use an explicitly empty `emoji` to remove the reaction. The worker rejects missing emoji, ordinary text and unbounded emoji input; Meta validates the exact Unicode emoji sequence. The target must belong to the same chat, must not itself be a reaction, must not have been deleted, and must be no more than 30 days old. A reaction uses `message_id` as its target rather than a separate quoted-reply field.

## Locations and contacts

`whatsapp::cloud::messages::send_location`:

```json
{
  "chat_id": "+15551234567",
  "latitude": -8.0476,
  "longitude": -34.8770,
  "name": "Meeting point",
  "address": "Recife, Brazil",
  "reply_to_message_id": "wamid.incoming"
}
```

Both coordinates are required; zero is valid. Latitude must be within -90..90 and longitude within -180..180. Name and address are optional and limited to 1024 characters each by worker policy.

`whatsapp::cloud::messages::send_contacts`:

```json
{
  "chat_id": "+15551234567",
  "contacts": [
    {
      "name": {"formatted_name": "Alex Morgan", "first_name": "Alex", "last_name": "Morgan"},
      "phones": [{"phone": "+15557654321", "wa_id": "15557654321", "type": "WORK"}],
      "emails": [{"email": "alex@example.com", "type": "WORK"}],
      "org": {"company": "Example", "department": "Support", "title": "Specialist"},
      "birthday": "1990-01-20"
    }
  ]
}
```

The typed contact object also supports addresses and website URLs. Formatted name is required. Meta allows up to 257 contacts per message; the worker additionally caps contact JSON at 65536 bytes. Birthday must be a real `YYYY-MM-DD` date. Recipient devices may omit some contact metadata. Website URLs are display data and are never fetched by the worker.

## Lists and reply buttons

`whatsapp::cloud::messages::send_list`:

```json
{
  "chat_id": "+15551234567",
  "text": "Choose a delivery option.",
  "header": {"type": "text", "text": "Delivery"},
  "footer": "Select one option",
  "button": "Choose",
  "sections": [
    {
      "title": "Available services",
      "rows": [
        {"id": "standard", "title": "Standard", "description": "Three business days"},
        {"id": "express", "title": "Express", "description": "Next business day"}
      ]
    }
  ]
}
```

Lists allow 1..10 sections and 1..10 rows **in total**. Row IDs must be unique. Current Meta documentation permits 4096 body characters, 20 button characters, 24 section/row-title characters, 200 row-ID characters and 72 description characters. List headers support text only.

`whatsapp::cloud::messages::send_buttons`:

```json
{
  "chat_id": "+15551234567",
  "text": "Would you like the complete report?",
  "header": {"type": "document", "media_id": "987654321"},
  "buttons": [
    {"id": "send_report", "title": "Send report"},
    {"id": "cancel", "title": "Cancel"}
  ]
}
```

Reply buttons allow 1..3 buttons with unique IDs and titles. IDs allow 256 characters and titles 20. The body allows 1024 characters. Optional headers can be text (60 characters) or an uploaded image/video/document media ID; hosted media links and local paths are not accepted. Optional footer text is limited to 60 characters.

## URL buttons and location requests

`whatsapp::cloud::messages::send_cta_url`:

```json
{
  "chat_id": "+15551234567",
  "text": "View the report in your browser.",
  "display_text": "Open report",
  "url": "https://example.com/reports/october"
}
```

The destination must be an HTTP(S) URL without embedded credentials, at most 2048 bytes (worker policy). It is sent as a button destination and is never fetched by the worker. Body, header and footer limits match reply buttons.

`whatsapp::cloud::messages::request_location`:

```json
{"chat_id": "+15551234567", "text": "Share your pickup location."}
```

The location request supports a body of up to 1024 characters, without header or footer.

## Media carousels

`whatsapp::cloud::messages::send_carousel`:

```json
{
  "chat_id": "+15551234567",
  "text": "Browse the available reports.",
  "cards": [
    {
      "header": {"type": "image", "media_id": "111111111"},
      "text": "October report",
      "display_text": "Read report",
      "url": "https://example.com/reports/october"
    },
    {
      "header": {"type": "image", "media_id": "222222222"},
      "text": "September report",
      "display_text": "Read report",
      "url": "https://example.com/reports/september"
    }
  ]
}
```

Carousels require 2..10 cards with image/video media-ID headers. Main body is limited to 1024 characters. Optional card body is limited to 160 characters and two line breaks. Each card uses either a single URL button or `buttons: [{"id": "...", "title": "..."}]` for quick replies. The worker limits quick replies to 1..3 per card; button kind and count must match across all cards. Card indices are generated in input order. No top-level header or footer is accepted.

The payload follows Meta's current carousel examples, which use `card.type: "cta_url"` even in the quick-reply example and `action.buttons[].type: "quick_reply"` for quick-reply buttons. Media-ID headers use the shared documented Header/Media objects. These variants are covered by local HTTP contract tests; no live carousel was sent to resolve the documentation ambiguity.

## Flow launch

`whatsapp::cloud::messages::send_flow`:

```json
{
  "chat_id": "+15551234567",
  "text": "Choose an appointment time.",
  "flow_id": "123456789",
  "flow_cta": "Book appointment",
  "flow_token": "appointment-42",
  "mode": "published",
  "flow_action": "navigate",
  "screen": "START",
  "data": {"customer_name": "Alex"}
}
```

Exactly one of `flow_id` or `flow_name` is required. The worker sends Flow message version `3`. Mode defaults to `published`; `draft` requires explicit selection. `navigate` is the default action and requires the entry screen. Optional input data must be a non-empty JSON object of at most 16384 bytes (worker policy). A `data_exchange` launch omits screen/data and uses the Flow's already configured data endpoint; this function does not implement that endpoint. The Flow must exist and meet Meta's publication/eligibility requirements.

The worker caps CTA labels at Meta's recommendation of 30 characters, Flow names at 255, screen IDs at 256, and correlation tokens at 4096. The last three are worker bounds. Body/header/footer limits are 1024/60/60 characters, with text headers only.

All standalone interactive, contact and location sends accept optional `reply_to_message_id`; it becomes `context.message_id` in the Graph request. All message sends in this document accept optional `recipient_identity_key_hash`. These fields are unrelated to native JIDs or sessions.

## Official sources

Documentation checked on 2026-10-02:

- [Read receipts](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/mark-message-as-read) and [typing indicators](https://developers.facebook.com/documentation/business-messaging/whatsapp/typing-indicators).
- [Reactions](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/reaction-messages), [contacts](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/contacts-messages), and [locations](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/location-messages).
- [Lists](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/interactive-list-messages), [reply buttons](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/interactive-reply-buttons-messages), [URL buttons](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/interactive-cta-url-messages), and [location requests](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/location-request-messages).
- [Media carousels](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/interactive-media-carousel-messages) and [message API object reference](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/message-api).
- [Sending Flows](https://developers.facebook.com/documentation/business-messaging/whatsapp/flows/guides/sendingaflow) and [message timing](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/send-messages).
