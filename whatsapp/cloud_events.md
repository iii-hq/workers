Filtered WhatsApp Cloud API messages, delivery statuses, and optional account/group/calling webhook events. Publishes verified events after durable deduplication; messages are also saved to the local archive.

## Register a subscription

1. Register a callback function that accepts the event object below. Replace `automation::on-whatsapp-message` with that function's ID and the example `chat_ids` with the contacts you want to monitor.
2. Call `engine::register_trigger` in the `default` namespace with the following JSON. Use an initialized caller connection in the callback function's namespace. `trigger_namespace` selects this WhatsApp provider, not the callback namespace. With the CLI, `III_NAMESPACE` sets the connection namespace; `--namespace default` routes the call to the engine function. For a newly connected Go SDK client, first wait until `engine::workers::list` shows that client in the intended namespace; `Connect` can return before its namespace is announced.

```json
{
  "trigger_type": "whatsapp::cloud::events",
  "trigger_namespace": "<provider-namespace>",
  "function_id": "automation::on-whatsapp-message",
  "config": {
    "chat_ids": ["+15550000009"],
    "message_types": ["text"]
  },
  "metadata": {"workflow": "customer-support"}
}
```

3. Save the returned `id`. Call `engine::registered-triggers::info` with `{"id":"<returned-id>"}` and check that `status` is `active` and `function` identifies the intended callback and namespace. Active confirms the provider binding; it does not guarantee the callback exists. Registration does not replay old messages.
4. Remove the subscription with `engine::unregister_trigger` and `{"id":"<returned-id>"}`. Function-path registrations survive the calling connection closing; explicitly unregister them when finished. Callback metadata is separate from the event payload. Callback return values do not send replies to WhatsApp.

## Filter configuration

Every field in `config` is optional. Always supply a JSON object; `{}` receives all incoming messages for the configured Cloud account. Unknown fields are rejected. Populated fields combine with AND; values within each list combine with OR.

| Field | Values and behavior |
| --- | --- |
| `kinds` | List: `message.received`, `message.status`, `webhook.received`. Omitted, null or empty defaults to only `message.received`. |
| `chat_ids` | Contact phone numbers such as `+15550000009`, business-scoped IDs such as `BR.ENT.Abc123`, or `group:<raw Cloud group ID>`. Use IDs from `whatsapp::cloud::chats::list`. Native JIDs are not supported. |
| `sender_ids` | List of sender IDs in the same format. Applies to incoming messages. |
| `message_types` | List of exact raw Meta types, such as `text`, `image`, `audio`, `interactive` or `button`. Names must start with a lowercase letter and contain 1–64 lowercase letters, digits or underscores. |
| `statuses` | List: `sent`, `delivered`, `read`, `failed`. Also set `kinds` to include `message.status`. |
| `fields` | Exact Meta webhook fields, such as `messages`, `message_template_status_update`, `calls`, or `group_lifecycle_update`. Administrative events require `kinds: ["webhook.received"]`. |
| `events` | Exact administrative event names, such as `APPROVED`, `group_create`, or a calling event name. |
| `group_ids` | Raw Cloud group IDs without the `group:` prefix. Matches group messages, receipts, and metadata events. |
| `participant_ids` | Normalized participants identified by group delivery/read receipts. Also select `message.status`. |
| `business_account_id` | Exact WABA ID, a numeric string of at most 32 digits. |
| `phone_number_id` | Exact Phone Number ID, a numeric string of at most 32 digits. |

Empty lists impose no restriction except the received-only default for `kinds`. Empty account ID strings impose no restriction. Each list allows at most 128 values; the entire filter is limited to 64 KiB. Status events have no sender or message type, so populated `sender_ids` or `message_types` exclude them. To receive read and failed receipts for one contact, replace `config` above with:

```json
{
  "kinds": ["message.status"],
  "chat_ids": ["+15550000009"],
  "statuses": ["read", "failed"]
}
```

## Callback payload

```json
{
  "kind": "message.received",
  "business_account_id": "123",
  "phone_number_id": "456",
  "message_id": "wamid.example",
  "chat_id": "+15550000009",
  "sender_id": "+15550000009",
  "message_type": "text",
  "text": "Hello",
  "timestamp_ms": 1700000000000,
  "data": {
    "id": "wamid.example",
    "from": "15550000009",
    "timestamp": "1700000000",
    "type": "text",
    "text": {"body": "Hello"}
  }
}
```

`message.status` uses the same account, message, chat and timestamp fields plus `status`; sender, message type and text may be absent. `timestamp_ms` is Meta's timestamp in milliseconds. `data` contains the single original message/status object, not the whole webhook. Events are capped at 64 KiB; oversized content uses `data: {"content_omitted":true}` with a clipped preview.

## Delivery and setup

Configure Cloud API credentials and the Meta webhook first; `whatsapp::cloud::status` reports local setup. This trigger does not configure Meta or subscribe to linked-device events (`whatsapp::events`). Local sends do not emit `message.received`; Meta receipts can emit `message.status`.

Duplicate messages and identical status updates are suppressed within retained deduplication history. Delivery is best effort, without callback retries or replay; queue overflow, shutdown or callback failures can lose notifications. Reconcile current retained message state with `whatsapp::cloud::chats::list` and `whatsapp::cloud::messages::list`. Reconcile administrative state with its corresponding management API; the message archive does not store account-event history.

## Administrative and group subscriptions

For template status changes, register a callback with this filter:

```json
{
  "trigger_type": "whatsapp::cloud::events",
  "trigger_namespace": "<provider-namespace>",
  "function_id": "automation::on-template-status",
  "config": {
    "kinds": ["webhook.received"],
    "fields": ["message_template_status_update"],
    "events": ["APPROVED", "REJECTED"]
  }
}
```

`webhook.received` has `event_id` (a deterministic hash), `field`, optional
`event`, and sanitized `data`. Its `message_id` and `chat_id` are empty.
`phone_number_id` is empty for WABA-wide events without a phone identity;
a nonempty phone filter excludes those events. `timestamp_ms` is Meta's entry
or nested event timestamp, or zero when Meta provides none. No receipt time is
substituted. Duplicate detection hashes the canonical event and supplied time.

Group metadata and calling collections are split into individual events while
retaining their nested `groups` or `calls` object inside `data`. Coexistence
history/state/echo payloads can be subscribed to by field, but are not imported
as ordinary inbox messages. This does not provision coexistence or initiate
history synchronization.

For a Cloud group, use:

```json
{
  "kinds": ["message.received", "message.status"],
  "group_ids": ["Y2FwaV9ncm91cDoxMjM="]
}
```

Group messages use `chat_id: "group:<id>"`, retain the participant as `sender_id`,
and include raw `group_id`. Delivery/read receipts for an individual participant
include `participant_id`; they are deduplicated per participant. They do not
mark the entire group conversation as delivered/read. These subscriptions use
the official Cloud Groups API, not linked-device group JIDs. Meta account
eligibility and webhook field subscriptions are still required.

See [Meta webhook fields](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/overview)
and [group webhook contracts](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/webhooks).
