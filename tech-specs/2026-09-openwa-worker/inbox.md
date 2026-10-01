# WhatsApp conversations and participating groups in ADE

The page uses a compact session selector and Mensagens, Grupos and Conexão tabs.
Conversation and group lists share a list/detail layout, with a back action in
narrow panels. Reading is passive: it does not send messages, start sessions,
download media or acknowledge messages as read. Sending and requesting older
history require explicit actions. Group membership uses the existing native
groups APIs; the page adds search, member/admin details and navigation to the
group's stored messages.

## Local archive

Each existing session owns a separate private `inbox.db`. SQLite stores only
normalized display fields, independently of native credentials. Incoming native
messages are captured through the durability hook; streamed history sync and
successful text/reply submissions also feed the archive. Pagination, search and
deduplication work while disconnected. Generation guards prevent asynchronous
callbacks from restoring data after logout, deletion or account changes.

`max_inbox_messages` defaults to 10,000 and can be changed live (1–100,000).
Retention drops oldest rows. View-once bodies, media keys and raw protobufs are
excluded. Temporary-message expiration removes the stored body. Archive capture
and history synchronization timestamps explain the available coverage.

New functions:

- `whatsapp::chats::list`: session, optional search/cursor/limit; chat summaries.
- `whatsapp::messages::list`: session/chat, optional cursor/limit; newest-first
  stored messages and continuation cursor.
- `whatsapp::messages::request_history`: session/chat/count; an asynchronous
  request anchored by the oldest retained message, requiring a connection.

Default list limits are 100 chats and 50 messages, with a maximum of 200 per
page. History requests default to 50 and allow at most 100. Phone history is
not guaranteed to be available; an empty chat has no anchor for a request.
Phone-number and LID addresses remain separate chat/sender identities when the
native source uses different aliases. Device suffixes and own-message echoes
deduplicate, but this archive does not implement a cross-alias contact directory.

## Upgrade and validation

The previous worker retained only a bounded in-memory message event ring. An
offline CLI import accepts a private saved `events::poll` page and an existing
session ID, uses the same normalization as live capture, and returns counts
without connecting to iii or WhatsApp. Session envelopes cannot cross accounts.
This supports preserving available events during the first upgrade. It cannot
recover event bodies already discarded by the old process.

Validation covers persistence, deduplication, pagination, retention, privacy,
account lifecycle, CLI isolation, offline engine APIs and configuration reloads.
Frontend checks cover data merging/request scopes, group browsing, message
reading, mobile navigation, disconnected states and explicit-only actions.
Live deployment preserves the paired account and restores its prior connection;
no messages or group mutations are used for validation.

Verified on 2026-09-30: 73 Rust tests including the isolated real-engine test,
57 frontend tests, strict UI lint, Clippy and formatting passed. Browser checks
covered desktop/mobile and disconnected states without page errors. The live
upgrade preserved the existing session, imported its 3 available events and
listed 63 groups. One explicit older-history request returned 50 messages,
bringing the available conversation to 53 stored messages. The original event
snapshot was removed after successful migration.
