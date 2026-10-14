# Cloud commerce, Brazil payment messages and marketing

These official Cloud functions use the existing account configuration, authenticated HTTP client, message archive and filtered webhook trigger. They do not use native linked-device sessions.

All message responses use `{ "message_id": "wamid.example", "chat_id": "+15551234567", "status": "accepted", "message_status": null }`. Acceptance does not confirm delivery, payment, template approval or business eligibility. Follow `whatsapp::cloud::events` for received messages and delivery/failure statuses. POST timeouts return `cloud_outcome_unknown`; the worker never retries, charges a card, generates a Pix code, confirms payment, or sends an alternate message automatically.

## Catalogs and products

These messages require inventory in a Meta catalog connected to the WABA and an open customer service window. `product_retailer_id` is the catalog's SKU/content ID, not the numeric Graph object ID.

`whatsapp::cloud::messages::send_catalog`:

```json
{
  "chat_id": "+15551234567",
  "text": "Browse our catalog.",
  "thumbnail_product_retailer_id": "report-october",
  "footer": "Reports and subscriptions"
}
```

`thumbnail_product_retailer_id` is optional. Without it Meta chooses a catalog product image.

`whatsapp::cloud::messages::send_product`:

```json
{
  "chat_id": "+15551234567",
  "catalog_id": "123456789",
  "product_retailer_id": "report-october",
  "text": "Here is the report you requested."
}
```

The single-product body/footer are optional; headers are not accepted. Other commerce message bodies are required and limited to 1024 characters; optional footers allow 60 characters.

`whatsapp::cloud::messages::send_products`:

```json
{
  "chat_id": "+15551234567",
  "text": "Choose a report.",
  "header_text": "Available reports",
  "catalog_id": "123456789",
  "sections": [
    {"title": "Monthly", "product_retailer_ids": ["report-october", "report-september"]}
  ]
}
```

Supports 1..10 sections and at most 30 products in total. The text header is required, with a maximum of 60 characters; section titles allow 24. Catalog IDs and SKU values have worker bounds of 128 and 256 characters respectively. Meta may omit missing catalog products or reject a message if none match.

`whatsapp::cloud::messages::send_product_carousel`:

```json
{
  "chat_id": "+15551234567",
  "text": "Browse the latest reports.",
  "catalog_id": "123456789",
  "product_retailer_ids": ["report-october", "report-september"]
}
```

Supports 2..10 cards from one catalog. The worker generates consecutive `card_index` values and `type: "product"` cards. Header, footer and separate buttons are not accepted.

## Address requests in India

`whatsapp::cloud::messages::request_address` is available only to businesses based in India and their Indian customers. It requires an Indian phone recipient and `country: "IN"`; the worker does not infer eligibility from the account configuration.

```json
{
  "chat_id": "+919876543210",
  "text": "Where should we deliver your order?",
  "country": "IN",
  "values": {"name": "Alex", "city": "Mumbai", "in_pin_code": "400063"},
  "saved_addresses": [
    {"id": "home", "value": {"name": "Alex", "address": "Example street", "city": "Mumbai", "in_pin_code": "400063"}}
  ]
}
```

Typed values include name, phone number, Indian pin code, house/floor/tower/building, address, landmark, city and state. `validation_errors` uses the same field names with explanatory text. Worker bounds are 8192 JSON bytes per values object and at most 10 saved addresses. Actual address selections arrive through the existing messages webhook.

## Brazil payment requests

`whatsapp::cloud::messages::send_order_details` maps to `interactive.type: "order_details"`, `action.name: "review_and_pay"`, with `payment_type: "br"` and currency `BRL`. Account and regional eligibility remain enforced by Meta.

A simplified request with an existing PSP payment link:

```json
{
  "chat_id": "+5511987654321",
  "text": "Review your report order.",
  "reference_id": "invoice-42-attempt-1",
  "goods_type": "digital-goods",
  "total_amount": {"value": 5000, "offset": 100},
  "payment_settings": [
    {"type": "payment_link", "payment_link": {"uri": "https://pay.example.com/invoice-42"}}
  ]
}
```

Use a unique `reference_id` for every order-details message. It must contain 1..60 ASCII letters, digits, underscores, dashes or dots. `goods_type` is `digital-goods` or `physical-goods`. An amount of 5000 with offset 100 represents BRL 50.00. Amounts are exact integer cents bounded to the JSON safe integer range, not floating-point currency values. `offset` must always be 100.

The optional `order` object adds itemization:

```json
{
  "status": "pending",
  "catalog_id": "123456789",
  "items": [
    {"retailer_id": "report-october", "name": "October report", "amount": {"value": 2500, "offset": 100}, "quantity": 2}
  ],
  "subtotal": {"value": 5000, "offset": 100},
  "tax": {"value": 0, "offset": 100}
}
```

The worker validates these identities before HTTP dispatch:

- Subtotal equals the sum of quantity multiplied by the item price, using `sale_amount` when supplied. Sale prices must be lower than original prices.
- Total equals subtotal plus tax and shipping minus discount.
- Totals/item prices are positive; quantities are positive integers. All intermediate calculations must fit the JSON safe integer range.

Tax is required for an itemized order, even when zero. Optional shipping and discount use `{value, offset, description}`; discounts additionally support `discount_program_name`. New order status defaults to `pending`. An optional expiration has an epoch-seconds **string** at least 300 seconds in the future and a description of up to 120 characters. Worker bounds are 100 items and 65536 JSON bytes for the order parameters. This interface omits image headers; Meta can select the image from the itemized catalog products, and simplified orders do not support image headers.

Payment settings are typed and limited to one per supported type:

```json
[
  {
    "type": "pix_dynamic_code",
    "pix_dynamic_code": {"code": "PSP_GENERATED_DYNAMIC_PIX_CODE", "merchant_name": "Example", "key": "billing@example.com", "key_type": "EMAIL"}
  },
  {"type": "payment_link", "payment_link": {"uri": "https://pay.example.com/invoice-42"}},
  {"type": "boleto", "boleto": {"digitable_line": "PSP_GENERATED_DIGITABLE_LINE"}},
  {"type": "offsite_card_pay", "offsite_card_pay": {"last_four_digits": "1234", "credential_id": "psp-credential-reference"}}
]
```

Pix key types are `CPF`, `CNPJ`, `EMAIL`, `PHONE` or `EVP`. The bank/PSP integration supplies the actual Pix code, boleto, link or credential reference. Never provide a full card number or security code. Offsite card pay only presents the customer's confirmation choice and returns it by webhook; a separate authorized PSP integration must perform settlement. HTTP(S) payment links are display destinations and are never fetched by this worker.

Worker bounds for payment settings are: Pix code 4096 characters, merchant/key 255, boleto line 128, credential reference 256, and payment URL 2048 bytes. These are worker policies, not asserted Meta limits.

## Order and payment status

`whatsapp::cloud::messages::send_order_status`:

```json
{
  "chat_id": "+5511987654321",
  "text": "Your payment has been confirmed.",
  "reference_id": "invoice-42-attempt-1",
  "order": {"status": "completed"},
  "payment": {"status": "captured", "timestamp": 1790982000}
}
```

At least one order or payment object is required. Order states: `pending`, `processing`, `partially_shipped`, `shipped`, `completed`, `canceled`. Payment states: `pending`, `captured`, `failed`. Meta enforces transitions. Only report `captured` after independently confirming payment with the PSP. The worker never derives a payment confirmation from message acceptance or delivery.

## Dedicated Marketing Messages API

`whatsapp::cloud::messages::send_marketing_template` uses `POST /{phone_number_id}/marketing_messages`:

```json
{
  "chat_id": "+15551234567",
  "name": "monthly_offer",
  "language": "en_US",
  "components": [{"type": "body", "parameters": [{"type": "text", "text": "Alex"}]}],
  "product_policy": "STRICT",
  "message_activity_sharing": false
}
```

Only approved marketing templates are supported by that endpoint; Meta verifies category and onboarding. `components` follows the existing bounded template-components contract. `product_policy` defaults to `STRICT`, avoiding Meta's implicit routing fallback to the standard Cloud API. Explicitly choose `CLOUD_API_FALLBACK` to permit that routing. Omitting `message_activity_sharing` preserves the WABA default; the worker does not enable sharing automatically. BSUID recipients are supported subject to Meta's documented optimization restrictions.

## Direct Send beta

`whatsapp::cloud::messages::send_direct`:

```json
{
  "chat_id": "+15551234567",
  "category": "utility",
  "type": "text",
  "text": "Your order has shipped.",
  "ttl_seconds": 600,
  "template_name": "order_shipped"
}
```

The explicit category is `utility`, `authentication` or `service`. Utility/authentication require access to Meta's beta; authentication and business-named templates have additional restrictions. Authentication requires a phone number, not a BSUID. Service messages still require an open service window.

Supported input types are `text`, `buttons` (1..3 standard reply buttons), and `cta_url` (one URL button), using the same typed header/footer/button fields as [interactive messages](cloud-interactive.md). Text allows 4096 characters; interactive bodies allow 1024. URL previews are not exposed because Direct Send does not support them. Media headers are access-restricted by Meta.

TTL is optional. Explicit utility TTL is 30..43200 seconds; authentication TTL is 30..900. It is rejected for service messages. Omission preserves Meta's category default. Optional `template_name` is utility-only, with 1..512 lowercase letters, digits or underscores. The wire field is `direct_send_config.template_name`; template-name collisions may fail asynchronously.

The worker submits exactly once. Meta documents its own retries during business-named template creation; these happen within Meta and cannot be disabled by the worker's HTTP retry policy. No local retry or template fallback is implemented. The current Direct Send documentation also mentions a mixed format with up to 10 buttons, but its referenced example page returned Page Not Found when checked. That unverified mixed-button wire format is not exposed here.

All sends above support optional `reply_to_message_id` and `recipient_identity_key_hash` using the existing Cloud message contracts.

## Official sources

Documentation checked on 2026-10-02:

- [Catalog messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/catalogs/catalog-messages), [single products](https://developers.facebook.com/documentation/business-messaging/whatsapp/catalogs/single-product-messages), [multiple products](https://developers.facebook.com/documentation/business-messaging/whatsapp/catalogs/multi-product-messages), and [product carousels](https://developers.facebook.com/documentation/business-messaging/whatsapp/catalogs/interactive-product-carousel-messages).
- [Address messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/address-messages).
- [Brazil orders](https://developers.facebook.com/documentation/business-messaging/whatsapp/payments/payments-br/orders), [Pix](https://developers.facebook.com/documentation/business-messaging/whatsapp/payments/payments-br/offsite-pix), [payment links](https://developers.facebook.com/documentation/business-messaging/whatsapp/payments/payments-br/payment-links), [boleto](https://developers.facebook.com/documentation/business-messaging/whatsapp/payments/payments-br/boleto), and [one-click payments](https://developers.facebook.com/documentation/business-messaging/whatsapp/payments/payments-br/one-click-payments).
- [Marketing sends](https://developers.facebook.com/documentation/business-messaging/whatsapp/marketing-messages/send-marketing-messages).
- [Direct Send categories](https://developers.facebook.com/documentation/business-messaging/whatsapp/direct-send/send-utility-and-authentication-messages), [message types](https://developers.facebook.com/documentation/business-messaging/whatsapp/direct-send/supported-message-types), [TTL](https://developers.facebook.com/documentation/business-messaging/whatsapp/direct-send/configure-message-ttl), and [business-named templates](https://developers.facebook.com/documentation/business-messaging/whatsapp/direct-send/business-named-templates).
