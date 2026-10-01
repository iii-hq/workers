# Native WhatsApp surface for an iii worker

Inspected `oxidezap/whatsapp-rust` commit `dd0763c4635b353c4ed52def39d030fb2f375f9a` and the 202-operation OpenWA OpenAPI snapshot on 2026-09-30. This is a source-level implementation map, not a claim that a worker implements or matches OpenWA. No WhatsApp account was connected and no live operation was tested.

Source paths below are relative to [the pinned upstream tree](https://github.com/oxidezap/whatsapp-rust/tree/dd0763c4635b353c4ed52def39d030fb2f375f9a). The inspected checkout was `/tmp/whatsapp-rust-native-check`; the OpenAPI snapshot was `/tmp/openwa-iii-research-20260930/openapi.json`.

## Integration decisions

- Use `whatsapp_rust::Bot` with a persistent backend, keep its `Arc<Client>` and `BotHandle` per session, and expose explicit iii request/response DTOs.
- Public accessors borrow the client: `groups()`, `contacts()`, `profile()`, `blocking()`, `chat_actions()`, `newsletter()`, `status()`, `labels()`, `voip()`. Call them inside the handler after retrieving the session's `Arc<Client>`.
- Network operations need a running, paired client. Use `wait_for_connected(Duration)` where waiting is intended; expose disconnected and unpaired states accurately.
- All 21 `GroupController` operations, four `ProfileController` operations, ten `ChannelController` operations, and both `CallController` operations have plausible native implementations. This establishes native building blocks only: OpenWA parameter/output compatibility and authorization semantics still need adapters and tests.
- Chats, message history, contacts, status history, label queries, search, and statistics need an application-owned read model populated from sync/events. A protocol client is not the OpenWA application database.
- Most feature response structs do **not** implement `serde::Serialize`. Do not use `json!(result)` blindly or encode Rust `Debug` strings as a public API.

## Exact useful signatures

These are signatures from public methods, with whitespace normalized. Unless marked synchronous, methods are `pub async fn`; the omitted receiver is `&self`. `Jid` is `whatsapp_rust::Jid`; `wa` is `whatsapp_rust::waproto::whatsapp`. Feature types and errors are re-exported by `whatsapp_rust::features` and many through crate root/prelude; check the selected import rather than importing private feature modules.

### Groups — `client.groups()`

Source: `src/features/groups.rs`; request types in `wacore/src/iq/groups.rs`.

| Public method signature | OpenWA operation or use |
| --- | --- |
| `list_participating() -> Result<Vec<GroupOverview>, GroupError>` | `SessionController_getGroups` |
| `fetch_metadata(jid: &Jid) -> Result<GroupMetadata, GroupError>` | `GroupController_findOne`, `getSettings` |
| `create_group(options: GroupCreateOptions) -> Result<CreateGroupResult, GroupError>` | `GroupController_create` |
| `get_invite_info(code: &str) -> Result<GroupMetadata, GroupError>` | `joinInfo` |
| `join_with_invite_code(code: &str) -> Result<JoinGroupResult, GroupError>` | `join` |
| `set_subject(jid: impl Into<Jid>, subject: GroupSubject) -> Result<(), GroupError>` | `setSubject` |
| `set_description(jid: impl Into<Jid>, description: Option<GroupDescription>, prev: PreviousDescription<'_>) -> Result<(), GroupError>` | `setDescription` |
| `leave(jid: impl Into<Jid>) -> Result<(), GroupError>` | `leave` |
| `add_participants(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `addParticipants` |
| `remove_participants(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `removeParticipants` |
| `promote_participants(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `promoteParticipants` |
| `demote_participants(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `demoteParticipants` |
| `get_membership_requests(jid: impl Into<Jid>) -> Result<Vec<MembershipRequest>, GroupError>` | `getMembershipRequests` |
| `approve_membership_requests(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `approveMembershipRequests` |
| `reject_membership_requests(jid: impl Into<Jid>, participants: &[Jid]) -> Result<Vec<ParticipantChangeResponse>, GroupError>` | `rejectMembershipRequests` |
| `get_invite_link(jid: impl Into<Jid>, reset: bool) -> Result<String, GroupError>` | `getInviteCode` / `revokeInviteCode`; result is a link, so adapt explicitly |
| `lookup_profile_picture(group_jid: &Jid, preview: bool, existing_id: Option<&str>) -> Result<ProfilePictureLookup, GroupError>` | `getPicture` |
| `set_profile_picture(group_jid: impl Into<Jid>, image_data: Vec<u8>) -> Result<SetProfilePictureResponse, GroupError>` | `setPicture` |
| `remove_profile_picture(group_jid: impl Into<Jid>) -> Result<SetProfilePictureResponse, GroupError>` | `deletePicture` |
| `set_locked(jid: impl Into<Jid>, locked: bool) -> Result<(), GroupError>` | `updateSettings`: restrict group info editing |
| `set_announce(jid: impl Into<Jid>, announce: bool) -> Result<(), GroupError>` | `updateSettings`: announcements |
| `set_ephemeral(jid: impl Into<Jid>, expiration: u32) -> Result<(), GroupError>` | `updateSettings`: disappearing duration in seconds |
| `set_membership_approval(jid: impl Into<Jid>, mode: MembershipApprovalMode) -> Result<(), GroupError>` | `updateSettings`: join approval |
| `set_member_add_mode(jid: impl Into<Jid>, mode: MemberAddMode) -> Result<(), GroupError>` | `updateSettings`: who may add members |
| `set_member_link_mode(jid: &Jid, mode: MemberLinkMode) -> Result<(), GroupError>` | Native additional setting |
| `set_group_history(jid: impl Into<Jid>, enabled: bool) -> Result<(), GroupError>` | Native additional setting |
| `set_limit_sharing(jid: &Jid, enabled: bool) -> Result<(), GroupError>` | Native additional setting |

`GroupCreateOptions::new(subject)` and `.with_participants(Vec<GroupParticipantOptions>)` are available; `GroupParticipantOptions::new(jid)` constructs a participant. Subject/description are validated string types. `PreviousDescription::Resolve` fetches the current token and is the safe default when metadata freshness is unknown; `Absent` asserts no previous description, and `Id(&str)` supports optimistic concurrency. Preserve `GroupError::DescriptionConflict` distinctly.

`GroupMetadata` provides `id`, optional `subject`, `description`, `description_id`, `participants`, `is_locked`, `is_announcement`, `ephemeral`, `membership_approval`, `member_add_mode`, and more. `GroupParticipant` exposes `jid`, `phone_number`, `lid`, `participant_type`, and `is_admin()` / `is_super_admin()`. Participant mutations can succeed at the IQ level while individual participants fail: project each `ParticipantChangeResponse` (`jid`, optional `status`, `error`, `phone_number`, `username`, `add_request`) and retain partial failure information.

### Contacts, profile, blocking

Sources: `src/features/{contacts,profile,blocking,chat_actions}.rs`; response definitions in `wacore/src/iq/{contacts,usync}.rs`.

| Accessor and public method signature | OpenWA mapping |
| --- | --- |
| `contacts().is_on_whatsapp(jids: &[Jid]) -> Result<Vec<IsOnWhatsAppResult>, ContactError>` | `ContactController_checkNumber`; also useful for PN/LID resolution |
| `contacts().get_user_info(jids: &[Jid]) -> Result<HashMap<Jid, UserInfo>, ContactError>` | `findOne` only for server metadata, not stored address-book fields |
| `contacts().lookup_profile_picture(jid: &Jid, preview: bool, existing_id: Option<&str>) -> Result<ProfilePictureLookup, ContactError>` | `getProfilePicture`; batch adapter for `getProfilePictures` |
| `contacts().get_profile_picture(jid: &Jid, preview: bool) -> Result<Option<ProfilePicture>, ContactError>` | Simpler picture lookup with less outcome detail |
| `contacts().find_by_username(username: &str, username_key: Option<&str>) -> Result<UsernameLookup, ContactError>` | Additional native lookup |
| `chat_actions().save_contact(jid: &Jid, full_name: Option<String>, first_name: Option<String>, save_on_primary_addressbook: bool) -> Result<(), AppStateError>` | `upsertContact` |
| `chat_actions().remove_contact(jid: &Jid) -> Result<(), AppStateError>` | `deleteContact` |
| `blocking().get_blocklist() -> Result<Vec<BlocklistEntry>, BlockingError>` | `getBlockedContacts` |
| `blocking().block(jid: &Jid) -> Result<(), BlockingError>` | `blockContact` |
| `blocking().unblock(jid: &Jid) -> Result<(), BlockingError>` | `unblockContact` |
| `blocking().is_blocked(jid: &Jid) -> Result<bool, BlockingError>` | Additional native query |
| `profile().set_push_name(name: &str) -> Result<(), ProfileError>` | `ProfileController_setName` |
| `profile().set_status_text(text: &str) -> Result<(), ProfileError>` | `setStatus`: profile About, not a story |
| `profile().set_profile_picture(image_data: Vec<u8>) -> Result<SetProfilePictureResponse, ProfileError>` | `setPicture` |
| `profile().remove_profile_picture() -> Result<SetProfilePictureResponse, ProfileError>` | `deletePicture` |

`IsOnWhatsAppResult` preserves `is_registered`, `jid`, optional `lid` and `pn_jid`, `is_business`, `verified_name`, `username`, and independent subprotocol errors. Only PN/LID JIDs are valid for this query. `get_user_info` provides status, picture ID, devices, business and username metadata; it does not enumerate all saved contacts. Do not invent a phone number for a LID without a returned mapping.

Contact save/remove mutations specifically require a bare PN JID. They do not accept LIDs, group JIDs, or device-specific JIDs.

`ProfilePictureLookup` distinguishes `Found(ProfilePicture)`, `Unchanged`, `NotFound`, `NotAuthorized`, and `RateOverlimit` and is non-exhaustive. `ProfilePicture` fields are `id`, `url`, optional `direct_path`, optional `hash`. `SetProfilePictureResponse` has `id: String`. Picture setters expect prepared image bytes, not a URL or a data URL string; own-profile docs specify a prepared JPEG.

### Privacy — direct `Client` methods

Source: `src/client/iq_ops.rs`; types accessible through `whatsapp_rust::privacy_settings`.

```rust
fetch_privacy_settings() -> Result<PrivacySettingsResponse, IqError>
set_privacy_setting(category: PrivacyCategory, value: PrivacyValue)
    -> Result<SetPrivacySettingResponse, IqError>
set_privacy_disallowed_list(category: PrivacyCategory, update: DisallowedListUpdate)
    -> Result<SetPrivacySettingResponse, IqError>
set_default_disappearing_mode(duration: u32) -> Result<(), IqError>
```

`PrivacyCategory::is_valid_value` checks category/value combinations. Disallowed lists apply only to `Last`, `Profile`, `Status`, and `GroupAdd`; preserve the returned dhash for subsequent updates. These are native additions, not evidence of a corresponding OpenWA controller in the inspected snapshot.

### Chats — `client.chat_actions()`

Source: `src/features/chat_actions.rs`. All following methods return `Result<(), AppStateError>`.

```rust
archive_chat(jid: &Jid, message_range: Option<SyncActionMessageRange>)
unarchive_chat(jid: &Jid, message_range: Option<SyncActionMessageRange>)
pin_chat(jid: &Jid)
unpin_chat(jid: &Jid)
mute_chat(jid: &Jid)
mute_chat_until(jid: &Jid, mute_end_timestamp_ms: i64)
unmute_chat(jid: &Jid)
lock_chat(jid: &Jid)
unlock_chat(jid: &Jid)
mark_chat_as_read(jid: &Jid, read: bool, message_range: Option<SyncActionMessageRange>)
delete_chat(jid: &Jid, delete_media: bool, message_range: Option<SyncActionMessageRange>)
clear_chat(jid: &Jid, delete_starred: bool, delete_media: bool,
    message_range: Option<SyncActionMessageRange>)
star_message(chat_jid: &Jid, participant_jid: Option<&Jid>, message_id: &str, from_me: bool)
unstar_message(chat_jid: &Jid, participant_jid: Option<&Jid>, message_id: &str, from_me: bool)
delete_message_for_me(chat_jid: &Jid, participant_jid: Option<&Jid>, message_id: &str,
    from_me: bool, delete_media: bool, message_timestamp: Option<i64>)
set_user_status_mute(jid: &Jid, muted: bool)
```

These cover the mutating `SessionController` chat operations and `MessageController_starMessage`. They do not provide chat listing or message retrieval. Build correct ranges using public helper `message_range(last_message_timestamp: i64, last_system_message_timestamp: Option<i64>, messages: Vec<(wa::MessageKey, i64)>) -> SyncActionMessageRange`. Public `message_key(id: impl Into<String>, remote_jid: &Jid, from_me: bool, participant: Option<&Jid>) -> wa::MessageKey` constructs keys. App-state operations depend on paired/synchronized app-state keys; do not report success if upstream returns an error.

Labels similarly have native mutation methods: `create_label(label_id: &str, name: &str, color: i32)`, `delete_label(label_id: &str)`, `add_chat_label(label_id: &str, chat_jid: &Jid)`, and `remove_chat_label(label_id: &str, chat_jid: &Jid)` on `client.labels()`, all asynchronous and returning `Result<(), AppStateError>`. Label listing and association queries need a read model.

### Channels/newsletters — `client.newsletter()`

Source: `src/features/newsletter.rs`. All methods below are asynchronous and return the listed value in `Result<T, NewsletterError>`.

| Public signature | `T` | OpenWA mapping |
| --- | --- | --- |
| `list_subscribed()` | `Vec<NewsletterMetadata>` | `ChannelController_findAll` (subscribed scope) |
| `create(name: &str, description: Option<&str>)` | `NewsletterMetadata` | `create` |
| `get_metadata(jid: &Jid)` | `NewsletterMetadata` | `findOne` |
| `join(jid: &Jid)` | `NewsletterMetadata` | `subscribe` |
| `leave(jid: &Jid)` | `()` | `unsubscribe` |
| `delete(jid: &Jid)` | `()` | `remove` |
| `get_messages(jid: impl Into<Jid>, count: u32, before: Option<u64>)` | `Vec<NewsletterMessage>` | `getMessages` |
| `set_admin_mute(jid: &Jid, muted: bool)` | `()` | `mute`: channel updates visible to followers |
| `demote_admin(jid: &Jid, user: &Jid)` | `()` | `demoteAdmin` |
| `change_owner(jid: &Jid, user: &Jid)` | `()` | `transferOwnership` |
| `update(jid: &Jid, name: Option<&str>, description: Option<&str>)` | `NewsletterMetadata` | Additional native operation |
| `set_picture(jid: &Jid, jpeg: &[u8])` | `NewsletterMetadata` | Additional native operation |
| `remove_picture(jid: &Jid)` | `NewsletterMetadata` | Additional native operation |
| `get_metadata_by_invite(invite_code: &str)` | `NewsletterMetadata` | Additional native operation |
| `set_follower_mute(jid: &Jid, muted: bool)` | `()` | Admin notification setting for follower activity |
| `send_reaction(jid: &Jid, server_id: u64, reaction: &str)` | `String` | Channel-specific reactions |
| `edit_message(jid: &Jid, message_id: impl Into<String>, new_content: wa::Message)` | `()` | Channel edit |
| `revoke_message(jid: &Jid, message_id: impl Into<String>)` | `()` | Channel revoke |

`NewsletterMetadata` exposes `jid`, `name`, optional `description`, `subscriber_count`, `verification`, `state`, optional picture/preview URLs, invite code, role, creation time, and mute state. `NewsletterMessage.message_id` is the wire ID for edits/revokes; `server_id` is the numeric pagination/reaction ID. They are not interchangeable. Preserve optional `message: Option<wa::Message>` for revoked/no-body messages. Most timestamps use seconds; `last_edit_timestamp_ms` explicitly uses milliseconds.

### Status — `client.status()`

Source: `src/features/status.rs`. All async methods return `Result<SendResult, SendError>`.

```rust
send_text(text: &str, background_argb: u32,
    font: wa::message::extended_text_message::FontType,
    recipients: &[Jid], options: StatusSendOptions)
send_image(upload: UploadResponse, thumbnail: Vec<u8>, caption: Option<&str>,
    recipients: &[Jid], options: StatusSendOptions)
send_video(upload: UploadResponse, thumbnail: Vec<u8>, duration_seconds: u32,
    caption: Option<&str>, recipients: &[Jid], options: StatusSendOptions)
send_raw(message: wa::Message, recipients: &[Jid], options: StatusSendOptions)
revoke(message_id: impl Into<String>, recipients: &[Jid], options: StatusSendOptions)
```

The synchronous `audience() -> Option<Arc<wa::sync_action_value::StatusPrivacyAction>>` returns the last synced audience. `None` means unknown. `StatusSendOptions.privacy` only writes a stanza mode; it **does not filter recipients**. Callers must supply and validate the intended recipient list, and retain it for revocation. `StatusSendOptions` also has optional `message_id`, `extra_stanza_nodes`, and `device_freshness`.

These map to text/image/video status send and delete. Voice status can use an audio `wa::Message` through `send_raw`; there is no dedicated `send_voice` convenience method. `StatusController_getStatuses`, `getContactStatus`, and `getStatusMedia` require retained received status messages/media metadata; profile About text is not a substitute.

### Media — direct `Client` methods and helpers

Sources: `src/{upload,download,media}.rs`. Unqualified `Result` here is `anyhow::Result`.

```rust
upload(data: Vec<u8>, media_type: MediaType, options: UploadOptions)
    -> Result<UploadResponse>
download(downloadable: &dyn Downloadable) -> Result<Vec<u8>>
download_from_params(params: &DownloadParams) -> Result<Vec<u8>>
download_to_writer<W: DownloadWriter + Send + 'static>(
    downloadable: &dyn Downloadable, writer: W) -> Result<W>
download_from_params_to_writer<W: DownloadWriter + Send + 'static>(
    params: &DownloadParams, writer: W) -> Result<W>
```

`DownloadParams::encrypted(direct_path: impl Into<String>, media_key: &[u8], file_sha256: &[u8], file_enc_sha256: &[u8], file_length: u64, media_type: MediaType) -> Self` constructs download metadata. `UploadOptions::new()` supplies defaults. Upload response fields include URL/direct path, 32-byte media key and SHA hashes, length, key timestamp, optional streaming sidecar. Keep the response within the send operation where possible; it contains decryption material.

Public synchronous builders `whatsapp_rust::media::{image_message,video_message,document_message,audio_message}(upload: UploadResponse, opts: <corresponding Options>) -> wa::Message` build media messages for `Client::send_message`. This provides protocol upload/download, not OpenWA's voice/video conversion queue, transcoding jobs, thumbnail generation, or arbitrary remote URL fetching. Those require additional implementation; no conversion success should be synthesized from a successful upload.

### Calls — `client.voip()`

Source: `src/client/voip.rs`; call-link types in `wacore/src/types/group_call.rs`.

```rust
reject(incoming: &IncomingCall) -> Result<(), CallError>
reject_call(call_id: &str, peer: &Jid, call_creator: &Jid) -> Result<(), CallError>
terminate(call_id: &str, peer: &Jid, call_creator: &Jid) -> Result<(), CallError>
create_call_link(media: CallLinkMedia) -> Result<CallLink, CallError>
```

Reject/terminate are available without enabling the media engine. `create_call_link` requires the opt-in `voip-control` feature; the crate's default features do not include it. `CallLinkMedia::{Audio,Video}` is typed. `CallLink` contains `token` and `media`, implements `Serialize`, and its synchronous `.url()` returns the actual WhatsApp call URL. Reject needs the call creator as well as peer and ID; retain incoming call event data instead of guessing. Broader real-time calls have builders and media backend/transport requirements and are outside OpenWA's two call endpoints.

## Lifetime and serialization constraints

Source: `src/bot.rs`, `src/client/{lifecycle,accessors}.rs`, `waproto/build.rs`, `wacore/binary/src/jid.rs`.

- `Bot::builder().with_backend(...).on_event(...).build().await` creates the client; `Bot::client() -> Arc<Client>` clones its shared handle; `Bot::spawn(self) -> BotHandle` consumes the bot and starts background work. `BotHandle::client() -> Arc<Client>` is also available. Store the handle to provide managed shutdown.
- `BotHandle::shutdown(self)` asynchronously disconnects, flushes the device snapshot, receipts and secrets, and waits for the run loop. `abort(&self)` skips that work and is explicitly an emergency escape hatch. Prefer graceful shutdown for normal session stop and worker termination.
- Lifecycle methods include async `logout(self: &Arc<Self>)`, `disconnect(self: &Arc<Self>)`, `reconnect(self: &Arc<Self>)`, `reconnect_immediately(self: &Arc<Self>)`, `pause(self: &Arc<Self>)`, and synchronous `resume(&self)`, `is_paused() -> bool`, `is_connected() -> bool`, `is_logged_in() -> bool`. Logout/disconnect return unit, so distinguish a requested transition from an observed event.
- `Client::run(self: &Arc<Self>)` and `run_with_reason(self: &Arc<Self>) -> RunCompletionReason` drive the long-running loop; do not start a duplicate loop for a session already launched through `Bot::spawn`.
- Pair/QR callbacks receive `(String, Duration)` and should retain expiry. Event callbacks receive `Arc<Event>` and `Arc<Client>`. Subscriptions returned by `subscribe` / `subscribe_handler` are lifetime handles; retain them or deliberately detach rather than immediately dropping them.
- `GroupMetadata`, `GroupOverview`, `ParticipantChangeResponse`, `UserInfo`, `IsOnWhatsAppResult`, `ProfilePicture`, `SetProfilePictureResponse`, `NewsletterMetadata`, `NewsletterMessage`, and `UploadResponse` lack serde derives in this pin. Project explicit DTO fields and preserve optional values and typed error distinctions.
- `Jid` serde is a structured value when enabled, not the conventional public API JID string. Use `jid.to_string()` deliberately in DTOs. Do not expect `HashMap<Jid, UserInfo>` to become a JSON object with acceptable string keys automatically.
- Generated `wa::Message` and other protobuf messages always implement `Serialize`, but `Deserialize` is behind the sibling `waproto` crate's `serde-deserialize` feature. Default `whatsapp-rust` dependency settings do not imply raw-message JSON deserialization. Prefer explicit message DTO builders; enable that feature intentionally if exposing a raw-message contract.
- Generated protobuf nested fields use `buffa::MessageField`, not ordinary `Option<Box<T>>`. Construct via the appropriate `MessageField::some(...)` or an upstream media/helper builder. Enum fields can be typed enums/open enums rather than `i32`.
- Non-exhaustive enums/structs require wildcard match arms or upstream builders. Units differ across APIs: muting uses milliseconds, group disappearing expiration uses seconds, status colors use ARGB integers, newsletter cursor IDs are `u64`.

## OpenWA gaps requiring an application layer

The 202-operation API includes much more than WhatsApp protocol operations. A native iii worker should declare its implemented function registry and document unsupported operations explicitly.

| OpenWA area | Native client boundary |
| --- | --- |
| API key CRUD/validation, audit, global settings | Worker/application authentication and persistent configuration, not WhatsApp client functions |
| Session CRUD/config/proxy, force kill, QR/pairing | Build a per-session manager, persisted config and callback state; native lifecycle is only the underlying mechanism |
| Webhook CRUD/testing/delivery failures | iii event routing or a real webhook dispatcher with persisted delivery/retry state |
| Automation rules, message templates, bulk batch status/cancel | Application records and job execution; a send method alone does not implement the endpoint |
| Contact/chat/message/status enumeration, history, reactions, media search | Durable sync/event projections and query APIs; server feature queries only cover specific cases |
| Label reads and chat-label lookups | Maintain app-state projections; native label mutations exist |
| Statistics and Prometheus metrics | Worker counters/projections and an exporter; `Client::stats()` is client operational telemetry, not OpenWA business statistics |
| Infrastructure engine discovery/update checks/restart/export/import | Worker deployment/storage control with separately defined semantics |
| Voice/video conversion status and jobs | Explicit transcoding dependencies and job storage; upload/download does not convert codecs |
| Plugins install/update/config/health, URL packages | No drop-in OpenWA runtime. Upstream Rust `plugins` is a build-time native client host, not OpenWA's installable plugin API |
| Plugin ingress/redrive and integration-instance CRUD | Requires a separately implemented integration execution/data model, not a WhatsApp protocol wrapper |
| Catalog and product sends | Upstream `client.business()` has catalog/product APIs; map and validate separately before advertising these endpoints |

No runtime OpenWA plugin installation, Node/browser execution, container-per-session behavior, protocol reimplementation, or full API parity follows from this map.
