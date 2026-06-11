@pure
Feature: session::update_message — streaming deltas and edited output

  Contract (session-manager.md § session::update_message): replaces the
  content (and optionally details) of an existing message entry. Each
  successful update increments the entry's revision (echoed on the
  event, monotonic per entry — consumers keep the highest). With
  expected_revision set, a mismatch writes nothing and returns
  { updated: false, revision } with NO event. This is the primitive the
  harness streams assistant deltas through.

  Background:
    Given a bare session
    And an empty assistant message appended to "s_001"

  # Prevents: revision stagnation — if revisions don't increase,
  # last-write-wins reconciliation in consumers breaks and stale
  # snapshots overwrite newer ones.
  Scenario: streaming growth increments the revision each update
    Given a binding "b1" on "session::message_updated" delivering to "ui::recv" with config:
      """
      {}
      """
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001",
        "content": [{ "type": "text", "text": "Hel" }] }
      """
    Then the response field "updated" is true
    And the response field "revision" is 1
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001",
        "content": [{ "type": "text", "text": "Hello world" }] }
      """
    Then the response field "revision" is 2
    And function "ui::recv" received 2 "session::message_updated" deliveries
    And delivery 0 to "ui::recv" has "revision" = 1
    And delivery 0 to "ui::recv" has "message.content.0.text" = "Hel"
    And delivery 1 to "ui::recv" has "revision" = 2
    And delivery 1 to "ui::recv" has "message.content.0.text" = "Hello world"
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.revision" is 2
    And the response field "entry.message.content.0.text" is "Hello world"

  # Prevents: updates rewriting history's position — the entry keeps its
  # creation timestamp (ordering anchor) while the event carries the
  # update time.
  Scenario: an update never changes the entry's creation timestamp
    Given the clock advances by 5000 ms
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001",
        "content": [{ "type": "text", "text": "x" }] }
      """
    And I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.timestamp" is 1000000

  # Prevents: lost-update races — two writers with the same baseline
  # must not both win; the loser gets updated:false and the current
  # revision, with no event emitted for the failed attempt.
  Scenario: expected_revision mismatch writes nothing and fires nothing
    Given a binding "b1" on "session::message_updated" delivering to "ui::recv" with config:
      """
      {}
      """
    Given I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001",
        "content": [{ "type": "text", "text": "winner" }], "expected_revision": 0 }
      """
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001",
        "content": [{ "type": "text", "text": "loser" }], "expected_revision": 0 }
      """
    Then the call succeeds
    And the response field "updated" is false
    And the response field "revision" is 1
    And function "ui::recv" received 1 "session::message_updated" delivery
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.message.content.0.text" is "winner"

  # Prevents: edited function output losing its structured details
  # (the pruned-content + compacted_at pattern from the harness).
  Scenario: details are replaced on function_result messages
    Given I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "function_result", "function_call_id": "c1", "function_id": "f::g",
                     "content": [{ "type": "text", "text": "long output" }],
                     "details": { "full": true }, "is_error": false, "timestamp": 5 } }
      """
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "[pruned]" }],
        "details": { "compacted_at": 1000123 } }
      """
    Then the response field "updated" is true
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    Then the response field "entry.message.details.compacted_at" is 1000123
    And the response field "entry.message.content.0.text" is "[pruned]"

  # Prevents: details silently landing on roles that have no details
  # field (they would vanish on the next read).
  Scenario: details on a user message are rejected
    Given a user message "plain" appended to "s_001"
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [], "details": { "x": 1 } }
      """
    Then the call fails with code "session/details_not_supported"

  # Prevents: bookkeeping entries being rewritten through the message
  # surface.
  Scenario: updating a custom entry is rejected
    Given a custom entry of type "compaction" appended to "s_001"
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002", "content": [] }
      """
    Then the call fails with code "session/invalid_entry_kind"

  # Prevents: updates against missing entries/sessions failing silently
  # or fabricating entries.
  Scenario: unknown entry and unknown session are rejected
    When I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "ghost", "content": [] }
      """
    Then the call fails with code "session/entry_not_found"
    When I call "session::update_message" with:
      """
      { "session_id": "s_404", "entry_id": "e_001", "content": [] }
      """
    Then the call fails with code "session/not_found"
