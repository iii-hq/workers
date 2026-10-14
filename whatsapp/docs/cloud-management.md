# Cloud API account management

All functions on this page use the configured `cloud.business_account_id`,
`cloud.phone_number_id`, access token and API version. They do not use a linked
device session. There is no arbitrary Graph URL, phone ID or WABA ID argument.
Template operations that address a Graph template ID first verify that it belongs
to the configured WABA, with a bounded collection read (at most 100 pages).

These are authenticated worker functions: grant callers only the function IDs
they need using the engine's invocation authorization. Management permissions
on the Meta token are required in addition to messaging permissions where Meta
documents them. Phone registration, OTP, deregistration, PIN changes and webhook
subscriptions affect the real account when invoked.

The tables below show JSON request bodies. Call the IDs exactly as shown through
the engine. `{}` means an empty request, not omitted authorization.

## Templates

| Function ID | Example request | Result |
| --- | --- | --- |
| `whatsapp::cloud::templates::create` | `{"name":"report_ready","language":"en_US","category":"UTILITY","components":[{"type":"BODY","text":"Your requested report is ready."}]}` | Template `id`, review `status` and `category` returned by Meta. |
| `whatsapp::cloud::templates::get` | `{"template_id":"123456789"}` | Template fields including `status`, `components`, `quality_score`, `rejected_reason` when returned. |
| `whatsapp::cloud::templates::update` | `{"template_id":"123456789","components":[{"type":"BODY","text":"Your updated report is ready."}]}` | `{"success":true}` means the edit was accepted, not approved for sending. |
| `whatsapp::cloud::templates::delete` | `{"name":"report_ready","template_id":"123456789"}` | `{"success":true}`. Omitting `template_id` deletes all languages with that name. |
| `whatsapp::cloud::templates::unpause` | `{"template_id":"123456789"}` | `{"success":true}` if Meta allows manual unpausing. |

The existing `whatsapp::cloud::templates::list` and
`whatsapp::cloud::messages::send_template` remain available. Create/update support
`category`, `components`, `parameter_format`, `allow_category_change`,
`cta_url_link_tracking_opted_out` and `message_send_ttl_seconds`. Create additionally
requires `name` and `language`; update requires `template_id` and at least one
editable field. The categories are `UTILITY`, `MARKETING` and `AUTHENTICATION`.
Meta validates component-specific rules, language availability, examples, review
eligibility, TTL rules and edit limits. Component objects preserve documented
template structures instead of inventing a universal component shape.

Worker bounds are 100 components, 32 top-level properties per component, a
128 KiB JSON body and a TTL of at most 2,592,000 seconds. These are worker guardrails,
not a claim that every template category accepts these maxima. Meta may enforce
stricter values. Editing can require another review. No manual pause function is
exposed because an independently documented contract was not established.

**Archival gap:** Meta's management overview says bulk archive/unarchive endpoints
exist, but the linked archival page and current Message Template API reference did
not provide their endpoint/payload contracts when checked. No archive/unarchive
function is exposed. The documented automatic policy archives templates inactive
for 12 months and deletes archived templates after 28 days. Use WhatsApp Manager
for recovery until an independently documented API contract is available.
[Template management](https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/template-management),
[Message Template API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-account/message-template-api),
[pausing](https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/template-pausing),
[archival](https://developers.facebook.com/documentation/business-messaging/whatsapp/templates/template-archival).

## Business profile and phone number

| Function ID | Example request | Result |
| --- | --- | --- |
| `whatsapp::cloud::profile::get` | `{}` | `{"profile":{"about":"Support", "websites":["https://example.com"]}}` |
| `whatsapp::cloud::profile::update` | `{"about":"Customer support","email":"support@example.com","websites":["https://example.com"],"vertical":"PROF_SERVICES"}` | `{"success":true}` |
| `whatsapp::cloud::phone_numbers::get` | `{"fields":["id","verified_name","name_status","throughput"]}` | Requested fields on the configured phone. |
| `whatsapp::cloud::phone_numbers::list` | `{"limit":50}` | `{"items":[{"id":"123456789"}],"next_cursor":null}` |
| `whatsapp::cloud::phone_numbers::request_code` | `{"code_method":"SMS","language":"en_US"}` | Requests a real OTP by `SMS` or `VOICE`; `{"success":true}`. |
| `whatsapp::cloud::phone_numbers::verify_code` | `{"code":"123456"}` | `{"success":true}` |
| `whatsapp::cloud::phone_numbers::register` | `{"pin":"123456"}` | `{"success":true}` |
| `whatsapp::cloud::phone_numbers::deregister` | `{}` | `{"success":true}`; the number can no longer use Cloud API until registered again. |
| `whatsapp::cloud::phone_numbers::set_pin` | `{"pin":"654321"}` | `{"success":true}` |
| `whatsapp::cloud::phone_numbers::set_display_name` | `{"new_display_name":"Example Support"}` | `{"success":true}`; follow review status and re-register explicitly after approval. |

Profile update also accepts `address`, `description` and `profile_picture_handle`.
The picture handle must come from Meta's **Resumable Upload API**, not the ordinary
Cloud document/media upload ID. Profile updates only send supplied fields; empty
strings and empty website arrays explicitly clear eligible fields. Website URLs
are passed to Meta as profile content, never fetched by this worker.

The profile field bounds enforced by the worker are 139 characters for about,
256 for address, 512 for description, 128 for email, two websites of at most
256 characters each, and 8,192 characters for a picture handle. Meta remains the
authority for acceptable profile content. Phone details use an explicit field
allowlist; name status and other optional fields can depend on Meta eligibility.

Registration optionally accepts `data_localization_region`, limited to Meta's
documented region codes in the function schema. It explicitly enables local
storage; later changes require deregistration and registration. Meta limits
registration/deregistration to 10 requests in a moving 72-hour window.
Deregistration is unavailable for numbers in WhatsApp Business app coexistence.
There is no phone deletion function: deletion is performed in WhatsApp Manager.
[Business profile API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/whatsapp-business-profile-api),
[phone numbers](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/phone-numbers),
[registration](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/registration),
[two-step verification](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/two-step-verification),
[display names](https://developers.facebook.com/documentation/business-messaging/whatsapp/display-names).

## WABA webhook subscriptions

| Function ID | Example request | Result |
| --- | --- | --- |
| `whatsapp::cloud::subscriptions::list` | `{"limit":50}` | `items` containing `whatsapp_business_api_data` and an optional `override_callback_uri`; `next_cursor`. |
| `whatsapp::cloud::subscriptions::subscribe` | `{}` | Subscribes the app represented by the current token, using the app's default callback. |
| `whatsapp::cloud::subscriptions::subscribe` | `{"override_callback_uri":"https://example.com/webhooks/whatsapp-cloud/my-project-whatsapp-go"}` | Subscribes with a WABA callback override and the already configured `cloud.verify_token`. |
| `whatsapp::cloud::subscriptions::unsubscribe` | `{}` | Removes this app's WABA subscription; webhook delivery for it stops. |

Mutation results are `{"success":true}`. The callback must be a public HTTPS URL
without embedded credentials or token query parameters. No caller-supplied token
is accepted, and the configured verify token is never returned. Meta verifies the
callback; this operation does not start a tunnel or implement app-level webhook
field configuration. App-token subscriptions and phone-specific callback overrides
are separate APIs and are not exposed by these WABA functions. A Quick Tunnel URL
can be supplied explicitly, but its temporary URL must be updated when it changes.
[Subscribed Apps API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-account/subscribed-apps-api).

## Commerce, QR links and conversational components

| Function ID | Example request | Result |
| --- | --- | --- |
| `whatsapp::cloud::commerce::get` | `{}` | `{"settings":{"is_cart_enabled":true,"is_catalog_visible":false}}` |
| `whatsapp::cloud::commerce::update` | `{"is_cart_enabled":true,"is_catalog_visible":true}` | `{"success":true}`; at least one field is required. |
| `whatsapp::cloud::qr_codes::create` | `{"prefilled_message":"I need help","generate_qr_image":"SVG"}` | `code`, `prefilled_message`, `deep_link_url`, optional `qr_image_url`. |
| `whatsapp::cloud::qr_codes::list` | `{"limit":25}` | `items` and `next_cursor`. |
| `whatsapp::cloud::qr_codes::get` | `{"code":"4O4YGZEG3RIVE1"}` | `items` (empty if absent), `next_cursor`. |
| `whatsapp::cloud::qr_codes::update` | `{"code":"4O4YGZEG3RIVE1","prefilled_message":"I need sales help"}` | Updated QR record. |
| `whatsapp::cloud::qr_codes::delete` | `{"code":"4O4YGZEG3RIVE1"}` | `{"success":true}`; permanently retires that link. |
| `whatsapp::cloud::conversational_components::get` | `{}` | `{"settings":{"prompts":["Help"],"commands":[]}}`; `settings` is null if Meta omits it. |
| `whatsapp::cloud::conversational_components::update` | `{"prompts":["Help","Order status"],"commands":[{"command_name":"help","command_description":"Contact support"}]}` | `{"success":true}` |

QR prefilled messages allow 140 characters and image formats `PNG` or `SVG`; list
pages allow at most 25 records. These are public business links, not device login
QR codes. The worker returns Meta URLs without downloading their contents.

Conversational components support four prompts of at most 80 characters and
30 commands with names of at most 32 characters and descriptions of at most
256. Empty arrays clear the supplied component. Meta does not support emojis;
the worker rejects symbol/emoji characters and limits command names to ASCII
letters, digits and underscores. Cart visibility is independent of catalog
inventory; these functions do not create or edit products.
[Commerce settings](https://developers.facebook.com/documentation/business-messaging/whatsapp/catalogs/set-commerce-settings),
[QR and short links](https://developers.facebook.com/documentation/business-messaging/whatsapp/qr-codes),
[QR API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/whatsapp-business-qr-code-management-api),
[conversational components](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/conversational-components).

## Aggregate analytics

`whatsapp::cloud::analytics::get` reads configured WABA aggregates, not the local
message archive. This complete example requests one UTC day:

```json
{
  "field": "analytics",
  "start": 1790812800,
  "end": 1790899200,
  "granularity": "DAY"
}
```

Replace the timestamps with the intended period before invoking it. Result:
`{"field":"analytics","data":{"data_points":[{"sent":2,"delivered":2}]}}`.
No raw Graph `fields` expression is accepted. The worker constructs the expression
from validated field names, integer UTC timestamps and enum values.

| `field` | `granularity` | Additional fields |
| --- | --- | --- |
| `analytics` | `HALF_HOUR`, `DAY`, `MONTH` | None in this wrapper. |
| `conversation_analytics` | `HALF_HOUR`, `DAILY`, `MONTHLY` | Optional `metric_types`: `COST`, `CONVERSATION`. |
| `pricing_analytics` | `HALF_HOUR`, `DAILY`, `MONTHLY` | None in this wrapper. |
| `call_analytics` | `HALF_HOUR`, `DAILY`, `MONTHLY` | Optional `metric_types`: `COUNT`, `COST`, `AVERAGE_DURATION`. |
| `template_analytics` | `DAILY` | Required `template_ids` (1–10); optional `metric_types`: `COST`, `CLICKED`, `DELIVERED`, `READ`, `SENT`. |
| `template_group_analytics` | `DAILY` | Required `template_group_ids` (1–10); optional lowercase `metric_types`: `cost`, `clicked`, `delivered`, `read`, `sent`. |
| `group_analytics` | `DAILY` | Required `group_ids` (one) and `metric_types`: `SENT`, `DELIVERED`, `READ`, `PARTICIPANTS_JOINED`, `PARTICIPANTS_LEFT`. |

`start` must precede `end`. The worker enforces Meta's one-year messaging,
conversation and pricing lookback, and the 90-day template/template-group/group
lookback. It also limits calling requests to one year as a worker policy; the
calling guide does not establish the same retention promise. End timestamps may
be at most five minutes ahead to tolerate clock skew. Meta controls dataset
availability, opt-in, timezone normalization and billing restrictions. There are
no custom breakdown filters, Marketing Messages attribution metrics, local-time
date strings or automatic analytics opt-in. Large responses retain the shared
2 MiB HTTP response limit. [Analytics documentation](https://developers.facebook.com/documentation/business-messaging/whatsapp/analytics).

## Blocking and identity safety

| Function ID | Example request | Result |
| --- | --- | --- |
| `whatsapp::cloud::blocking::list` | `{"limit":50}` | `items` of blocked user IDs, `next_cursor`. |
| `whatsapp::cloud::blocking::block` | `{"users":["+15551234567"]}` | `block_users` with `added_users` and/or `failed_users`. |
| `whatsapp::cloud::blocking::unblock` | `{"users":["+15551234567"]}` | `block_users` with `removed_users` and/or `failed_users`. |
| `whatsapp::cloud::identity::get` | `{}` | `{"settings":{"enable_identity_key_check":true}}`, or `settings:null` when Meta omits identity settings. |
| `whatsapp::cloud::identity::update` | `{"enable_identity_key_check":true}` | `{"success":true}` |

Blocking accepts at most 1,000 unique international phone numbers per call. Meta
requires a user to have contacted the business in the previous 24 hours and does
not allow blocking another business account; the blocklist limit is 64,000 users.
BSUID-based blocking is not assumed by this wrapper. For a bulk error such as
`139100`, some users may already have been blocked/unblocked. The shared client
returns a structured `cloud_api_error` and numeric Meta diagnostics, discarding
arbitrary upstream error text. Reconcile with `blocking::list` before explicitly
retrying. Blocklist responses can provide a cursor without a `paging.next` URL;
the worker preserves that cursor and an empty page marks exhaustion.

Identity updates send the exact guide payload:
`{"user_identity_change":{"enable_identity_key_check":true}}` to `/PHONE/settings`.
The generic Settings reference separately describes an `enabled` field, and its
GET schema does not guarantee an identity record. Therefore reads preserve only
these documented identity booleans when returned and otherwise report null;
they never expose calling/SIP credentials. The worker does not infer successful
identity verification from enabling a setting. Authenticate a changed user through
another trusted channel before explicitly sending sensitive data again. There is
no invented identity acknowledgement endpoint or automatic trust reset.
[Block users](https://developers.facebook.com/documentation/business-messaging/whatsapp/block-users),
[identity change check](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/phone-numbers#identity-change-check),
[Settings API](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/settings-api).

## Error, pagination and retry contracts

All management failures use the worker's structured invocation errors:
`invalid_request`, `cloud_disabled`, `cloud_not_configured`, `cloud_api_error`,
`cloud_resource_not_found`, `cloud_invalid_response`, `cloud_request_canceled` or
`cloud_outcome_unknown`. Numeric Meta error/subcode diagnostics are retained, not
raw error messages that could echo credentials. Missing mutation acknowledgements,
transport timeouts and unreadable mutation responses are ambiguous. The worker
does not repeat the mutation. Read back account state before deciding to retry.

`success:true` acknowledges an API operation; it does not establish template
approval, delivery of a message, display-name review approval or successful
off-channel customer authentication. Follow the relevant webhook/state instead.

List functions return `{items, next_cursor}`. Pass `next_cursor` as `after` in the
next explicit request. No Graph pagination URL or configured secret is returned.
Unless a smaller Meta limit is documented above, pages are limited to 100 items
by worker policy and cursors to 2,048 characters.
