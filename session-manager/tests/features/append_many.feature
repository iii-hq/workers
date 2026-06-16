@pure
Feature: session::append-many — append several message entries in order

  Contract (session-manager.md § session::append-many): messages are
  appended in request order, each chained to the previous; one
  session::message-added fires per entry, in order; the response carries
  every entry id plus last_entry_id; the batch is NOT idempotent (use
  session::append with entry_id where redelivery is possible).

  Background:
    Given a bare session

  # Prevents: batch order scrambling — a user/assistant exchange written
  # out of order corrupts the conversation for every consumer.
  Scenario: a batch preserves order and chains the parent links
    Given a binding "b1" on "session::message-added" delivering to "ui::recv" with config:
      """
      {}
      """
    When I call "session::append-many" with:
      """
      { "session_id": "s_001", "messages": [
          { "role": "user", "content": [{ "type": "text", "text": "one" }], "timestamp": 1 },
          { "role": "assistant", "content": [{ "type": "text", "text": "two" }],
            "stop_reason": "end", "model": "m", "provider": "p", "timestamp": 2 },
          { "role": "user", "content": [{ "type": "text", "text": "three" }], "timestamp": 3 }
        ] }
      """
    Then the call succeeds
    And the response field "entry_ids" is ["e_001","e_002","e_003"]
    And the response field "last_entry_id" is "e_003"
    And function "ui::recv" received 3 "session::message-added" deliveries
    And delivery 0 to "ui::recv" has "entry_id" = "e_001"
    And delivery 0 to "ui::recv" has "parent_id" = null
    And delivery 1 to "ui::recv" has "entry_id" = "e_002"
    And delivery 1 to "ui::recv" has "parent_id" = "e_001"
    And delivery 2 to "ui::recv" has "entry_id" = "e_003"
    And delivery 2 to "ui::recv" has "parent_id" = "e_002"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.message_count" is 3

  # Prevents: the batch detaching from existing history instead of
  # chaining onto the active leaf.
  Scenario: a batch chains onto the current active leaf
    Given a user message "root" appended to "s_001"
    When I call "session::append-many" with:
      """
      { "session_id": "s_001", "messages": [
          { "role": "user", "content": [], "timestamp": 2 }
        ] }
      """
    And I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    Then the response field "entry.parent_id" is "e_001"

  # Prevents: a batch landing under a parent that does not exist.
  Scenario: a batch under an unknown parent is rejected
    When I call "session::append-many" with:
      """
      { "session_id": "s_001", "parent_id": "ghost", "messages": [
          { "role": "user", "content": [], "timestamp": 1 }
        ] }
      """
    Then the call fails with code "session/parent_not_found"

  # Prevents: an empty batch fabricating a last_entry_id out of thin air.
  Scenario: an empty batch is rejected
    When I call "session::append-many" with:
      """
      { "session_id": "s_001", "messages": [] }
      """
    Then the call fails with code "session/empty_batch"

  # Prevents: anyone assuming append_many dedupes — it must not; the
  # documented redelivery-safe path is append with entry_id.
  Scenario: append_many is not idempotent by design
    When I call "session::append-many" with:
      """
      { "session_id": "s_001", "messages": [
          { "role": "user", "content": [{ "type": "text", "text": "dup" }], "timestamp": 1 }
        ] }
      """
    And I call "session::append-many" with:
      """
      { "session_id": "s_001", "messages": [
          { "role": "user", "content": [{ "type": "text", "text": "dup" }], "timestamp": 1 }
        ] }
      """
    And I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 2
