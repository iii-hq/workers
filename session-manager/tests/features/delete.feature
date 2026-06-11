@pure
Feature: session::delete — delete a session and its entries

  Contract (session-manager.md § session::delete): removes the meta,
  every entry, and the active-leaf pointer; fires session::deleted.
  Deleting an unknown session returns { deleted: false } and fires
  nothing. Forks are copies, so deleting a source never affects
  sessions forked from it (covered in fork.feature).

  # Prevents: half-deleted sessions — meta gone but entries leaking, or
  # the delete not announcing itself.
  Scenario: delete removes everything and fires deleted
    Given a bare session
    And a user message "one" appended to "s_001"
    And a binding "b1" on "session::deleted" delivering to "ui::deleted" with config:
      """
      {}
      """
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    Then the call succeeds
    And the response field "deleted" is true
    And function "ui::deleted" received 1 "session::deleted" delivery
    And delivery 0 to "ui::deleted" has "session_id" = "s_001"
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response is null
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response is null
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the call fails with code "session/not_found"

  # Prevents: phantom deleted events for sessions that never existed.
  Scenario: deleting an unknown session is a silent false
    Given a binding "b1" on "session::deleted" delivering to "ui::deleted" with config:
      """
      {}
      """
    When I call "session::delete" with:
      """
      { "session_id": "s_404" }
      """
    Then the call succeeds
    And the response field "deleted" is false
    And function "ui::deleted" received no deliveries

  # Prevents: double-deletes double-firing.
  Scenario: a second delete is a false no-op
    Given a bare session
    And a binding "b1" on "session::deleted" delivering to "ui::deleted" with config:
      """
      {}
      """
    Given I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "deleted" is false
    And function "ui::deleted" received 1 "session::deleted" delivery

  # Prevents: zombie history — re-ensuring a deleted id must yield a
  # truly fresh session, not resurrect old entries via the stale leaf.
  Scenario: a recreated session id starts from scratch
    Given a bare session
    And a user message "old world" appended to "s_001"
    And I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    When I call "session::ensure" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "created" is true
    And the response field "meta.message_count" is 0
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 0
    When I append a user message "new world" to "s_001"
    Then the response field "parent_id" is null

  # Prevents: tenancy-filtered consumers missing deletions of their own
  # sessions — the filter must evaluate against the metadata as of
  # deletion (the meta row is already gone when the event lands).
  Scenario: deleted events match metadata filters as-of-deletion
    Given a session created with:
      """
      { "metadata": { "owner": "u_1" } }
      """
    And a binding "b1" on "session::deleted" delivering to "obs::u1" with config:
      """
      { "metadata": { "owner": "u_1" } }
      """
    And a binding "b2" on "session::deleted" delivering to "obs::u2" with config:
      """
      { "metadata": { "owner": "u_2" } }
      """
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    Then function "obs::u1" received 1 delivery
    And function "obs::u2" received no deliveries
