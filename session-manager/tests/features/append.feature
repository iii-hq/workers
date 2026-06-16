@pure
Feature: session::append — append one entry, idempotent on entry_id

  Contract (session-manager.md § session::append): the entry id and
  parent are assigned by the worker (parent = current active leaf)
  unless provided; appending always moves the active leaf to the new
  entry; appending an entry_id that already exists is a NO-OP — the
  existing entry is returned and no event fires. That idempotency is
  what makes the harness's redelivered queue steps safe.

  Background:
    Given a bare session

  # Prevents: the first message of a session getting a phantom parent.
  Scenario: first append becomes the root
    When I append a user message "hello" to "s_001"
    Then the call succeeds
    And the response field "entry_id" is "e_001"
    And the response field "parent_id" is null
    And the response field "timestamp" is 1000000

  # Prevents: breaking the parent chain — the chain IS the transcript
  # order; a wrong parent silently reorders or orphans history.
  Scenario: appends chain from the active leaf
    Given a user message "one" appended to "s_001"
    When I append a user message "two" to "s_001"
    Then the response field "entry_id" is "e_002"
    And the response field "parent_id" is "e_001"

  # Prevents: message_count drifting from the real number of messages
  # (list UIs render it as the conversation size).
  Scenario: appends maintain message_count and updated_at
    Given the clock advances by 500 ms
    And a user message "one" appended to "s_001"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.message_count" is 1
    And the response field "meta.created_at" is 1000000
    And the response field "meta.updated_at" is 1000500

  # Prevents: re-running a durable harness step from duplicating
  # transcript entries — THE core idempotency guarantee of this worker.
  Scenario: appending an existing entry_id is a complete no-op
    Given a binding "b1" on "session::message-added" delivering to "ui::recv" with config:
      """
      {}
      """
    When I call "session::append" with:
      """
      { "session_id": "s_001", "entry_id": "turn1-user",
        "message": { "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 } }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001", "entry_id": "turn1-user",
        "message": { "role": "user", "content": [{ "type": "text", "text": "DIFFERENT" }], "timestamp": 2 } }
      """
    Then the call succeeds
    And the response field "entry_id" is "turn1-user"
    And the response field "parent_id" is null
    And function "ui::recv" received 1 "session::message-added" delivery
    When I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "turn1-user" }
      """
    Then the response field "entry.message.content.0.text" is "hi"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.message_count" is 1

  # Prevents: an idempotent replay yanking the active leaf backwards,
  # which would re-parent the next streamed message onto stale history.
  Scenario: an idempotent replay does not move the active leaf
    Given I call "session::append" with:
      """
      { "session_id": "s_001", "entry_id": "a",
        "message": { "role": "user", "content": [], "timestamp": 1 } }
      """
    And a user message "newer" appended to "s_001"
    When I call "session::append" with:
      """
      { "session_id": "s_001", "entry_id": "a",
        "message": { "role": "user", "content": [], "timestamp": 1 } }
      """
    And I append a user message "next" to "s_001"
    Then the response field "parent_id" is "e_001"

  # Prevents: explicit parent_id being honored for the entry but the
  # leaf staying behind (spec: appending with parent_id also moves it).
  Scenario: explicit parent_id creates a branch and moves the leaf
    Given a user message "one" appended to "s_001"
    And a user message "two" appended to "s_001"
    When I call "session::append" with:
      """
      { "session_id": "s_001", "parent_id": "e_001",
        "message": { "role": "user", "content": [{ "type": "text", "text": "branch" }], "timestamp": 3 } }
      """
    Then the response field "parent_id" is "e_001"
    When I append a user message "after-branch" to "s_001"
    Then the response field "parent_id" is "e_003"

  # Prevents: silently attaching entries to parents that don't exist
  # (a corrupted chain is unrecoverable for consumers).
  Scenario: appending under an unknown parent is rejected
    When I call "session::append" with:
      """
      { "session_id": "s_001", "parent_id": "nope",
        "message": { "role": "user", "content": [], "timestamp": 1 } }
      """
    Then the call fails with code "session/parent_not_found"

  # Prevents: writer-supplied correlation (turn_id) being lost between
  # the request and the event consumers stitch traces with.
  Scenario: origin is echoed on the emitted event
    Given a binding "b1" on "session::message-added" delivering to "ui::recv" with config:
      """
      {}
      """
    When I call "session::append" with:
      """
      { "session_id": "s_001", "origin": { "turn_id": "t_9" },
        "message": { "role": "user", "content": [], "timestamp": 1 } }
      """
    Then delivery 0 to "ui::recv" has "origin.turn_id" = "t_9"
    And delivery 0 to "ui::recv" has "session_id" = "s_001"
    And delivery 0 to "ui::recv" has "entry_id" = "e_001"
    And delivery 0 to "ui::recv" has "parent_id" = null
    And delivery 0 to "ui::recv" has "timestamp" = 1000000

  # Prevents: the storage layer mangling any of the four roles — one
  # transcript must carry user, assistant, function_result and custom.
  Scenario: all four message roles round-trip through storage
    When I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "user", "content": [{ "type": "text", "text": "q" }], "timestamp": 1 } }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": "a" }],
                     "stop_reason": "function_call", "model": "m1", "provider": "p1",
                     "usage": { "input": 10, "output": 5 }, "timestamp": 2 } }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "function_result", "function_call_id": "c1", "function_id": "f::g",
                     "content": [{ "type": "text", "text": "out" }], "details": { "code": 0 },
                     "is_error": false, "timestamp": 3 } }
      """
    And I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "custom", "custom_type": "ui_marker",
                     "content": [{ "type": "text", "text": "pinned" }], "display": "Pinned",
                     "timestamp": 4 } }
      """
    And I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 4
    And the response field "messages.0.message.role" is "user"
    And the response field "messages.1.message.role" is "assistant"
    And the response field "messages.1.message.stop_reason" is "function_call"
    And the response field "messages.1.message.usage.input" is 10
    And the response field "messages.2.message.role" is "function_result"
    And the response field "messages.2.message.details.code" is 0
    And the response field "messages.3.message.role" is "custom"
    And the response field "messages.3.message.display" is "Pinned"

  # Prevents: content-block corruption — every block kind (text, image,
  # thinking, function_call, function_result) must survive storage
  # byte-for-byte for providers to replay them.
  Scenario: all five content block kinds round-trip inside one message
    When I call "session::append" with:
      """
      { "session_id": "s_001", "message": { "role": "assistant", "content": [
          { "type": "text", "text": "hello" },
          { "type": "image", "mime": "image/png", "data": "aWlp" },
          { "type": "thinking", "text": "hmm", "signature": "sig_1" },
          { "type": "function_call", "id": "c1", "function_id": "shell::run", "arguments": { "cmd": "ls" } },
          { "type": "function_result", "function_call_id": "c1",
            "content": [{ "type": "text", "text": "README.md" }], "is_error": false }
        ], "stop_reason": "end", "model": "m1", "provider": "p1", "timestamp": 2 } }
      """
    And I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.message.content.0.text" is "hello"
    And the response field "entry.message.content.1.mime" is "image/png"
    And the response field "entry.message.content.1.data" is "aWlp"
    And the response field "entry.message.content.2.signature" is "sig_1"
    And the response field "entry.message.content.3.function_id" is "shell::run"
    And the response field "entry.message.content.3.arguments.cmd" is "ls"
    And the response field "entry.message.content.4.function_call_id" is "c1"
    And the response field "entry.message.content.4.content.0.text" is "README.md"

  # Prevents: custom (bookkeeping) entries polluting message_count or
  # being dropped from events entirely.
  Scenario: appending a custom entry emits an event but does not count as a message
    Given a binding "b1" on "session::message-added" delivering to "ui::recv" with config:
      """
      {}
      """
    When I append a custom entry of type "compaction" to "s_001"
    Then the call succeeds
    And function "ui::recv" received 1 "session::message-added" delivery
    And delivery 0 to "ui::recv" has "custom.custom_type" = "compaction"
    And delivery 0 to "ui::recv" has no field "message"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.message_count" is 0

  # Prevents: ambiguous appends writing half a message and half a
  # bookkeeping entry.
  Scenario: exactly one of message / custom is required
    When I call "session::append" with:
      """
      { "session_id": "s_001" }
      """
    Then the call fails with code "session/invalid_request"
    When I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "user", "content": [], "timestamp": 1 },
        "custom": { "custom_type": "x", "data": {} } }
      """
    Then the call fails with code "session/invalid_request"

  # Prevents: appends silently creating sessions that were never declared.
  Scenario: appending to an unknown session is rejected
    When I append a user message "hi" to "s_404"
    Then the call fails with code "session/not_found"
