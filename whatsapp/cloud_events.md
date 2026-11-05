Verified WhatsApp Cloud API messages, delivery statuses, and account/group/calling webhook events. Each event is one call of this function after durable deduplication; incoming messages are saved to the local archive first. The function takes no args: observers select events with filters on its input fields.

## Observe events

1. Register a target function that accepts the event object below. Replace `automation::on-whatsapp-message` with that function's ID and the example `chat_id` with the contacts you want to monitor.
2. Observe this function with the following source. `namespace` selects this WhatsApp provider; the target resolves in the observing worker's namespace. `deliver: "output"` hands the target the event itself.

```json
{
  "function_id": "whatsapp::cloud::on-events",
  "namespace": "<provider-namespace>",
  "filters": {
    "input.kind": {"$eq": "message.received"},
    "input.chat_id": {"$in": ["+15550000009"]},
    "input.message_type": {"$eq": "text"}
  },
  "deliver": "output"
}
```

In Node: `iii.observe(source, 'automation::on-whatsapp-message', { metadata: { workflow: 'customer-support' } })`. In Go: `client.Observe(iii.Source{FunctionID: ..., Namespace: ..., Filters: ..., Deliver: iii.DeliverOutput}, "automation::on-whatsapp-message", iii.ObserverOptions{Metadata: ...})`. For a newly connected Go SDK client, first wait until `engine::workers::list` shows that client in the intended namespace; `Connect` can return before its namespace is announced.

3. Wait for the observer handle to be ready (`ready()` in Node, `Ready(ctx)` in Go). An active observer receives every later event its filters match; observing does not replay earlier events. The observer lives as long as the worker that created it: dispose its handle to stop receiving events. Observer metadata reaches the target as the call's metadata, separate from the event payload. Target return values do not send replies to WhatsApp.

## Filters

Each filter clause names an event field and an operator (`$eq`, `$in`, `$ne`, `$exists`, ...). Clauses combine with AND; `$in` lists alternatives. An observer without a filter on `input.kind` receives every kind, including delivery statuses and administrative events.

| Path | Values and presence |
| --- | --- |
| `input.kind` | `message.received`, `message.status`, or `webhook.received`. |
| `input.chat_id` | Contact phone numbers such as `+15550000009`, business-scoped IDs such as `BR.ENT.Abc123`, or `group:<raw Cloud group ID>`: the IDs `whatsapp::cloud::chats::list` returns. Empty on `webhook.received`. |
| `input.sender_id` | Sender of a `message.received` event, in the same format. |
| `input.message_type` | Exact raw Meta type of a `message.received` event, such as `text`, `image`, `audio`, `interactive` or `button`. |
| `input.status` | `sent`, `delivered`, `read`, or `failed`, on `message.status`. |
| `input.field` | Meta webhook field of a `webhook.received` event, such as `message_template_status_update`, `calls`, or `group_lifecycle_update`. |
| `input.event` | Administrative event name, such as `APPROVED`, `group_create`, or a calling event name. |
| `input.group_id` | Raw Cloud group ID without the `group:` prefix, on group messages, receipts, and group events. |
| `input.participant_id` | Group participant identified by a delivery or read receipt. |
| `input.business_account_id` | WABA ID, a numeric string. |
| `input.phone_number_id` | Phone Number ID, a numeric string; empty for WABA-wide administrative events. |

Filters compare exact values, so write contacts as `whatsapp::cloud::chats::list` returns them (`+15550000009`). A clause on a field an event lacks does not match it: status events have no sender or message type, so `$eq` or `$in` on `input.sender_id` or `input.message_type` excludes them. To receive read and failed receipts for one contact, use these filters:

```json
{
  "input.kind": {"$eq": "message.status"},
  "input.chat_id": {"$eq": "+15550000009"},
  "input.status": {"$in": ["read", "failed"]}
}
```

## Event payload

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

Configure Cloud API credentials and the Meta webhook first; `whatsapp::cloud::status` reports local setup. This function does not configure Meta or carry linked-device events (`whatsapp::on-events`). Local sends do not emit `message.received`; Meta receipts can emit `message.status`.

Duplicate messages and identical status updates are suppressed within retained deduplication history. Delivery is best effort, without target retries or replay; queue overflow, shutdown or target failures can lose events. Reconcile current retained message state with `whatsapp::cloud::chats::list` and `whatsapp::cloud::messages::list`. Reconcile administrative state with its corresponding management API; the message archive does not store account-event history.

## Administrative and group events

For template status changes, observe with these filters:

```json
{
  "input.kind": {"$eq": "webhook.received"},
  "input.field": {"$eq": "message_template_status_update"},
  "input.event": {"$in": ["APPROVED", "REJECTED"]}
}
```

`webhook.received` has `event_id` (a deterministic hash), `field`, optional
`event`, and sanitized `data`. Its `message_id` and `chat_id` are empty.
`phone_number_id` is empty for WABA-wide events without a phone identity;
a filter on a nonempty phone ID excludes those events. `timestamp_ms` is Meta's
entry or nested event timestamp, or zero when Meta provides none. No receipt
time is substituted. Duplicate detection hashes the canonical event and
supplied time.

Group metadata and calling collections are split into individual events while
retaining their nested `groups` or `calls` object inside `data`. Coexistence
history/state/echo payloads can be selected by field, but are not imported
as ordinary inbox messages. This does not provision coexistence or initiate
history synchronization.

For a Cloud group, use:

```json
{
  "input.kind": {"$in": ["message.received", "message.status"]},
  "input.group_id": {"$eq": "Y2FwaV9ncm91cDoxMjM="}
}
```

Group messages use `chat_id: "group:<id>"`, retain the participant as `sender_id`,
and include raw `group_id`. Delivery/read receipts for an individual participant
include `participant_id`; they are deduplicated per participant. They do not
mark the entire group conversation as delivered/read. These events use
the official Cloud Groups API, not linked-device group JIDs. Meta account
eligibility and webhook field subscriptions are still required.

See [Meta webhook fields](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/overview)
and [group webhook contracts](https://developers.facebook.com/documentation/business-messaging/whatsapp/groups/webhooks).
