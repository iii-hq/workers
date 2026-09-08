@pure
Feature: session attachments — keep the original files a message was sent with

  Contract: `session::put-attachment` stores an uploaded file's ORIGINAL
  bytes in the session's attachment store and returns a `type: "file"`
  content block that references them by `attachment_id`. The transcript
  never carries the bytes inline: a user message embeds the reference
  block beside the text the model reads, and `session::get-attachment` /
  `session::list-attachments` read the originals back. Uploading is
  event-silent and does not touch `message_count` or `updated_at` — the
  attachment surfaces through the message that references it. Deleting a
  session removes its attachments; forking copies exactly the attachments
  the copied history references, under the same ids, so no chip dies.

  Background:
    Given a bare session

  # Prevents: the original document being lost the moment it is turned
  # into text for the model — THE reason this surface exists.
  Scenario: an uploaded file reads back byte-for-byte with its metadata
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then the call succeeds
    And the response field "attachment.attachment_id" is "a_001"
    And the response field "attachment.session_id" is "s_001"
    And the response field "attachment.name" is "hello.txt"
    And the response field "attachment.mime" is "text/plain"
    And the response field "attachment.size" is 5
    And the response field "attachment.sha256" is "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    And the response field "attachment.created_at" is 1000000
    And the response field "block.type" is "file"
    And the response field "block.attachment_id" is "a_001"
    And the response field "block.name" is "hello.txt"
    And the response field "block.mime" is "text/plain"
    And the response field "block.size" is 5
    And an attachment folder exists for "s_001"
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the call succeeds
    And the response field "attachment.name" is "hello.txt"
    And the response field "data" is "aGVsbG8="
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "attachments" has length 1
    And the response field "attachments.0.attachment_id" is "a_001"

  # Prevents: an upload re-ordering session::list or waking message
  # subscribers before the message that carries it exists.
  Scenario: uploading is event-silent and leaves the session record alone
    Given a binding "b1" on "session::message-added" delivering to "obs::all" with config:
      """
      {}
      """
    And a binding "b2" on "session::meta-updated" delivering to "obs::all" with config:
      """
      {}
      """
    And the clock advances by 300 ms
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then the call succeeds
    And function "obs::all" received no deliveries
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.message_count" is 0
    And the response field "meta.updated_at" is 1000000

  # Prevents: a chip renderer paying for a 20 MB base64 body just to show a
  # name and a size.
  Scenario: a metadata-only read carries no bytes
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001", "include_data": false }
      """
    Then the call succeeds
    And the response field "attachment.size" is 5
    And the response field "data" is null

  # Prevents: a reader having to distinguish "no such session" from "no such
  # attachment" — both are simply not there, like session::get-message.
  Scenario: unknown attachments read as null
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_404" }
      """
    Then the call succeeds
    And the response is null
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_404", "attachment_id": "a_001" }
      """
    Then the call succeeds
    And the response is null
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_404" }
      """
    Then the call fails with code "session/not_found"

  # Prevents: the reference block being mangled on its way through the
  # transcript — the console re-hydrates chips from exactly these fields.
  Scenario: a message carrying a file block round-trips through the transcript
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    When I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "text", "text": "please read this" },
        { "type": "file", "attachment_id": "a_001", "name": "hello.txt", "mime": "text/plain", "size": 5 }
      ] } }
      """
    Then the call succeeds
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.message.content.0.text" is "please read this"
    And the response field "messages.0.message.content.1.type" is "file"
    And the response field "messages.0.message.content.1.attachment_id" is "a_001"
    And the response field "messages.0.message.content.1.name" is "hello.txt"
    And the response field "messages.0.message.content.1.mime" is "text/plain"
    And the response field "messages.0.message.content.1.size" is 5

  # Prevents: a fork rendering dead chips (same ids, no bytes) — and the
  # opposite waste of dragging every upload into every fork.
  Scenario: fork copies exactly the referenced attachments under the same ids
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "kept.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    And I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "orphan.txt", "mime": "text/plain", "data": "eHl6" }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "file", "attachment_id": "a_001", "name": "kept.txt", "mime": "text/plain", "size": 5 }
      ] } }
      """
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the call succeeds
    And the response field "session_id" is "s_002"
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_002", "attachment_id": "a_001" }
      """
    Then the response field "attachment.session_id" is "s_002"
    And the response field "attachment.name" is "kept.txt"
    And the response field "data" is "aGVsbG8="
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "attachments" has length 1
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "attachments" has length 2

  # Prevents: orphaned blobs surviving the session they belonged to — and a
  # recreated session id inheriting the previous owner's files.
  Scenario: deleting a session removes its attachments
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then an attachment folder exists for "s_001"
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "deleted" is true
    And no attachment folder exists for "s_001"
    When I call "session::ensure" with:
      """
      { "session_id": "s_001" }
      """
    And I call "session::list-attachments" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "attachments" has length 0

  # Prevents: attachments living only in a cache that a restart empties.
  Scenario: attachments survive a worker restart
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    When the worker restarts with the same data directory
    And I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the response field "attachment.name" is "hello.txt"
    And the response field "data" is "aGVsbG8="

  # Prevents: one oversized upload pinning unbounded memory on the worker;
  # the cap is operator configuration, applied per call.
  Scenario: an upload over the configured limit is refused
    Given the worker's attachment limit is 4 bytes
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then the call fails with code "session/attachment_too_large"
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "attachments" has length 0

  # Prevents: half-valid uploads landing as unreadable blobs.
  Scenario: malformed uploads are rejected before anything is written
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "bad.bin", "mime": "application/octet-stream", "data": "not base64!!" }
      """
    Then the call fails with code "session/invalid_request"
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "   ", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then the call fails with code "session/invalid_request"
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_404", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    Then the call fails with code "session/not_found"
    And no attachment folder exists for "s_001"

  # Prevents: the composer's attachments vanishing on a session switch or
  # reload while its text survives — the draft is one unit, text AND chips.
  Scenario: attachments parked with the draft read back on session::get
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "notes.txt", "mime": "text/plain", "data": "eHl6" }
      """
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "look at these", "attachment_ids": ["a_002", "a_001", "a_002"] }
      """
    Then the call succeeds
    And the response field "draft" is "look at these"
    And the response field "attachments" has length 2
    And the response field "attachments.0.attachment_id" is "a_002"
    And the response field "attachments.0.name" is "notes.txt"
    And the response field "attachments.1.attachment_id" is "a_001"
    And the response field "attachments.1.mime" is "image/png"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.draft" is "look at these"
    And the response field "meta.draft_attachments" has length 2
    And the response field "meta.draft_attachments.0.attachment_id" is "a_002"
    And the response field "meta.draft_attachments.1.size" is 5

  # Prevents: a keystroke-cadence text save silently dropping the chips.
  Scenario: a text-only draft save keeps the parked attachments
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "first", "attachment_ids": ["a_001"] }
      """
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "first draft, still typing" }
      """
    Then the response field "draft" is "first draft, still typing"
    And the response field "attachments" has length 1
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": null, "attachment_ids": [] }
      """
    Then the response field "draft" is null
    And the response field "attachments" has length 0
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response has no field "meta.draft"
    And the response has no field "meta.draft_attachments"

  # Prevents: a draft pointing at bytes that were never stored (a chip that
  # can never be rebuilt).
  Scenario: parking an unknown attachment id is rejected
    When I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "x", "attachment_ids": ["a_404"] }
      """
    Then the call fails with code "session/invalid_request"

  # Prevents: a fork inheriting the source's half-composed message.
  Scenario: fork does not carry the draft or its attachments
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "unsent", "attachment_ids": ["a_001"] }
      """
    And a user message "one" appended to "s_001"
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response has no field "meta.draft"
    And the response has no field "meta.draft_attachments"
    When I call "session::list-attachments" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "attachments" has length 0

  # Prevents: bytes of a chip removed from the composer lingering forever —
  # and a stale draft still pointing at them.
  Scenario: deleting a draft attachment frees the bytes and updates the draft
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "notes.txt", "mime": "text/plain", "data": "eHl6" }
      """
    And I call "session::set-draft" with:
      """
      { "session_id": "s_001", "draft": "x", "attachment_ids": ["a_001", "a_002"] }
      """
    When I call "session::delete-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the call succeeds
    And the response field "deleted" is true
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the response is null
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.draft_attachments" has length 1
    And the response field "meta.draft_attachments.0.attachment_id" is "a_002"
    When I call "session::delete-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the response field "deleted" is false
    When I call "session::delete-attachment" with:
      """
      { "session_id": "s_404", "attachment_id": "a_001" }
      """
    Then the call fails with code "session/not_found"

  # Prevents: deleting the bytes behind a sent message's file block — a chip
  # that could never be opened again.
  Scenario: an attachment referenced by a message cannot be deleted
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "hello.txt", "mime": "text/plain", "data": "aGVsbG8=" }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "file", "attachment_id": "a_001", "name": "hello.txt", "mime": "text/plain", "size": 5 }
      ] } }
      """
    When I call "session::delete-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the call fails with code "session/attachment_in_use"
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the response field "data" is "aGVsbG8="

  # Prevents: a browser's empty MIME string being stored as-is, leaving a
  # download with no content type.
  Scenario: a blank mime type falls back to application/octet-stream
    When I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mystery", "mime": "", "data": "aGVsbG8=" }
      """
    Then the response field "attachment.mime" is "application/octet-stream"
    And the response field "block.mime" is "application/octet-stream"

  # Prevents: opening a long, picture-heavy session dragging every image's
  # bytes into the transcript read — the reader opts out and fetches each
  # image through its stored original when it scrolls into view.
  Scenario: a transcript read can leave out the bytes of images it can refetch
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "text", "text": "two pictures" },
        { "type": "image", "mime": "image/png", "data": "aGVsbG8=", "attachment_id": "a_001" },
        { "type": "image", "mime": "image/png", "data": "eHl6" }
      ] } }
      """
    # Default: inline bytes exactly as written — the harness relies on it.
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages.0.message.content.1.data" is "aGVsbG8="
    And the response field "messages.0.message.content.1.attachment_id" is "a_001"
    And the response field "messages.0.message.content.2.data" is "eHl6"
    And the response has no field "messages.0.message.content.2.attachment_id"
    # Opt-out: linked images travel as references, unlinked ones stay inline
    # because there is nowhere else to load them from.
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "include_image_data": false }
      """
    Then the response field "messages.0.message.content.1.type" is "image"
    And the response field "messages.0.message.content.1.mime" is "image/png"
    And the response field "messages.0.message.content.1.data" is ""
    And the response field "messages.0.message.content.1.attachment_id" is "a_001"
    And the response field "messages.0.message.content.2.data" is "eHl6"
    When I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001", "include_image_data": false }
      """
    Then the response field "entry.message.content.1.data" is ""
    And the response field "entry.message.content.2.data" is "eHl6"
    When I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.message.content.1.data" is "aGVsbG8="
    # The stored file itself is untouched by the read.
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the response field "data" is "aGVsbG8="

  # Prevents: a lazily loaded picture chip pointing at bytes that were
  # deleted or never copied into the fork — an image block linked to a
  # stored original is a reference like any file block.
  Scenario: an image linked to a stored original is protected and forked like a file reference
    Given I call "session::put-attachment" with:
      """
      { "session_id": "s_001", "name": "mock.png", "mime": "image/png", "data": "aGVsbG8=" }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "user", "timestamp": 1, "content": [
        { "type": "image", "mime": "image/png", "data": "aGVsbG8=", "attachment_id": "a_001" }
      ] } }
      """
    When I call "session::delete-attachment" with:
      """
      { "session_id": "s_001", "attachment_id": "a_001" }
      """
    Then the call fails with code "session/attachment_in_use"
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "session_id" is "s_002"
    When I call "session::get-attachment" with:
      """
      { "session_id": "s_002", "attachment_id": "a_001" }
      """
    Then the response field "attachment.name" is "mock.png"
    And the response field "data" is "aGVsbG8="
