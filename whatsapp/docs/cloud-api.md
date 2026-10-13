# WhatsApp Cloud API coverage

[catalog.json](../catalog.json) is the exact request and response schema source
used for validation and engine discovery.

## Implemented contracts

| Capability | Contracts and examples |
| --- | --- |
| Document upload and send | [Document guide](meta-cloud-documents.md): separate upload and send by reusable media ID; explicit Markdown rejection. |
| Image, audio/voice, video and stickers | [Media guide](cloud-media.md): upload, authenticated bounded download, metadata, deletion, typed sends and voicemail announcement upload. |
| Contextual replies, identity hash, reactions, contacts, location, read receipts and typing | [Message actions](cloud-interactive.md): exact function IDs, optional reply fields, validation and real Meta payload shapes. |
| Interactive buttons, lists, URL buttons, location requests, carousels and Flow launches | [Interactive messages](cloud-interactive.md): separate typed operations. |
| Template create/get/update/delete/unpause | [Account management](cloud-management.md): existing template listing and sending remain available. |
| Profile, phone inventory/OTP/registration/PIN/display name, WABA subscriptions | [Account management](cloud-management.md): mutations explicitly target the configured account/phone. |
| Analytics, commerce settings, business QR links, conversational components, blocking and identity settings | [Account management](cloud-management.md): allowlisted fields and bounded pagination. |
| Flow lifecycle, assets, public-key setup and encrypted data exchange | [Flows](cloud-flows.md): signed RSA/AES endpoint with a configured iii business handler. |
| Official Cloud groups | [Groups and calls](cloud-groups-calls.md): management, invitations, join approvals, membership, group sends and pin/unpin. |
| Cloud calls | [Groups and calls](cloud-groups-calls.md): settings, permissions and signaling; caller supplies media transport. |
| Catalog/products, Brazil order/payment messages, India address collection | [Commerce](cloud-commerce.md): Meta messaging contracts; payment settlement belongs to the merchant/PSP. |
| Marketing Messages and Direct Send | [Commerce and marketing](cloud-commerce.md): dedicated route and bounded documented variants. |
| Account/template/group/call webhooks | [Event trigger](../cloud_events.md): opt-in `webhook.received`, field/event/group/participant filters and durable deduplication. |

Existing text/template/document sends, message archives, Cloud status, webhook
verification and Quick Tunnel operations keep their public IDs. New APIs reuse
the configured token, WABA, phone, Graph version, HTTP client and error format.
There is no generic arbitrary-Graph-request function.

## Operational boundaries

- Authorize invocation of the required function IDs in the engine. Management
  operations can change the real account when invoked; they are not previews.
- Files are bounded caller-provided bytes or authenticated media-ID downloads.
  No arbitrary host paths or caller-selected download URLs are accepted. URL
  fields in message/profile content are passed to Meta, not fetched by the worker.
- Mutations are attempted once. A timeout or missing acknowledgement can mean
  the operation succeeded remotely: `cloud_outcome_unknown` must not trigger an
  automatic resend. An accepted message ID is not a delivery receipt.
- Default triggers still emit incoming messages only. Administrative events
  require explicit opt-in. Group participant receipts do not mark the entire
  group as delivered/read; participant identity is preserved in event filtering.
- Meta controls service windows, template approval, permissions, country
  availability and feature eligibility. Local validation and fixture tests do
  not certify an account's access to groups, calling, marketing beta or payments.
- Native linked-device transport remains separate. Cloud groups use opaque
  `group_id` API arguments and `group:<id>` local archive IDs, never native JIDs.

## Remaining external and documented gaps

1. **Template archive/unarchive:** Meta's overview mentions these operations,
   but its linked current API documentation did not expose a usable endpoint and
   payload contract. No guessed endpoint is implemented. Manual pause is also
   not exposed. See the sources and exact distinction in the management guide.
2. **Onboarding and app setup:** Embedded Signup, account provisioning/migration,
   app-token webhook field subscriptions and phone-specific callback overrides
   are separate from the implemented WABA subscription functions. Existing
   credentials and Meta app configuration are still required.
3. **Resumable Upload API handles:** profile photos and template header examples
   accept the appropriate Meta handle, but this worker does not create a Meta
   app Resumable Upload session. Ordinary Cloud media IDs are not these handles.
4. **Coexistence history:** history/state/echo webhook fields can be delivered as
   filtered raw events; onboarding, history-sync initiation and automatic import
   into the archive are not implemented. Archive listing is not remote history.
5. **Calling and payments:** no WebRTC/SIP audio server, recording/transcription,
   PSP onboarding, payment processing or settlement is provided. Payment message
   contracts target Brazil; address collection targets India's eligible flow.
6. **Broader commerce and analytics:** Meta catalog inventory CRUD, ad/campaign
   management, custom analytics breakdowns, attribution datasets and analytics
   opt-in are outside these WhatsApp messaging wrappers.
7. **Direct Send and carousels:** only independently documented variants are
   exposed. The Direct Send mixed ten-button sample was unavailable; carousel
   differences in Meta's references are explained in the interactive guide.
8. **UI and API scope:** the ADE conversation composer is not a dashboard for
   every account-management function. Use typed engine functions for advanced
   management and orchestration.

The linked guides include official Meta sources, complete example requests,
response semantics, limits and eligibility details.

## ADE composer

The Cloud composer now offers **Attachment** alongside text and templates.
Choose a local browser file and optionally enter a caption for a document,
image or video. Selection checks the current worker byte limit; the worker
independently validates the bytes, MIME and codec requirements. Markdown is
rejected with an explicit TXT/PDF alternative; nothing is renamed or converted.
Upload begins only on **Send attachment**, followed by sending the returned
media ID. A failed or ambiguous send retains the upload for an explicit later
attempt. Navigating away before an upload completes prevents the subsequent
send. Cloud group conversations use the dedicated group send contracts.
