@pure
Feature: session::fork — copy history up to an entry into a new session

  Contract (session-manager.md § session::fork): copy-on-fork. Every
  entry on the root -> entry_id path is copied into a new session with
  FRESH entry ids (parent chain preserved structurally); the new
  session's active leaf is the copy of entry_id. After the fork the two
  sessions are fully independent — mutating or deleting one never
  affects the other. Fires session::created with forked_from set.

  Background:
    Given a session created with:
      """
      { "title": "main", "metadata": { "owner": "u_1" } }
      """
    And a user message "one" appended to "s_001"
    And an empty assistant message appended to "s_001"
    And a user message "three" appended to "s_001"

  # Prevents: forks sharing entry ids with the source — shared ids would
  # make an update to one transcript visibly corrupt the other.
  Scenario: fork copies the path with fresh ids and preserved structure
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_002", "title": "alt take" }
      """
    Then the call succeeds
    And the response field "session_id" is "s_002"
    And the response field "meta.title" is "alt take"
    And the response field "meta.forked_from" is "s_001"
    And the response field "meta.status" is "idle"
    And the response field "meta.message_count" is 2
    And the response field "meta.metadata.owner" is "u_1"
    When I call "session::messages" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_004"
    And the response field "messages.0.message.content.0.text" is "one"
    And the response field "messages.1.entry_id" is "e_005"
    When I call "session::get_message" with:
      """
      { "session_id": "s_002", "entry_id": "e_005" }
      """
    Then the response field "entry.parent_id" is "e_004"

  # Prevents: the fork's next append landing under the wrong leaf.
  Scenario: the fork's active leaf is the copy of the fork point
    Given I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    When I append a user message "fork continues" to "s_002"
    Then the response field "parent_id" is "e_005"

  # Prevents: forks defaulting to an empty title instead of inheriting
  # the source's.
  Scenario: fork without a title inherits the source title
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "meta.title" is "main"

  # Prevents: fork announcing itself without forked_from — dashboards
  # need it to draw the session tree.
  Scenario: fork fires session::created carrying forked_from
    Given a binding "b1" on "session::created" delivering to "obs::created" with config:
      """
      {}
      """
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    Then function "obs::created" received 1 "session::created" delivery
    And delivery 0 to "obs::created" has "session_id" = "s_002"
    And delivery 0 to "obs::created" has "forked_from" = "s_001"
    And delivery 0 to "obs::created" has "status" = "idle"

  # Prevents: cross-contamination — THE core copy-on-fork guarantee.
  # Writes to the source after the fork must never appear in the fork,
  # and vice versa.
  Scenario: source and fork are fully independent after the fork
    Given I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    When I append a user message "only in source" to "s_001"
    And I call "session::update_message" with:
      """
      { "session_id": "s_002", "entry_id": "e_004",
        "content": [{ "type": "text", "text": "edited only in fork" }] }
      """
    And I call "session::messages" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.message.content.0.text" is "edited only in fork"
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the response field "entry.message.content.0.text" is "one"
    And the response field "entry.revision" is 0

  # Prevents: deleting a source session cascading into its forks — the
  # spec explicitly promises forks survive.
  Scenario: deleting the source leaves the fork intact
    Given I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_003" }
      """
    When I call "session::delete" with:
      """
      { "session_id": "s_001" }
      """
    And I call "session::get" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "meta.forked_from" is "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "messages" has length 3

  # Prevents: forking from a mid-path entry dragging along entries from
  # sibling branches that are not on the root -> entry path.
  Scenario: only the path to the fork point is copied
    Given I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    And a user message "branch-b" appended to "s_001"
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_004" }
      """
    Then the response field "meta.message_count" is 2
    When I call "session::messages" with:
      """
      { "session_id": "s_002" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.1.message.content.0.text" is "branch-b"

  # Prevents: forks copying live revision counters — fresh copies start
  # a fresh revision space at 0.
  Scenario: copied entries restart their revision at 0
    Given I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "rev1" }] }
      """
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "e_002" }
      """
    And I call "session::get_message" with:
      """
      { "session_id": "s_002", "entry_id": "e_005" }
      """
    Then the response field "entry.revision" is 0
    And the response field "entry.message.content.0.text" is "rev1"

  # Prevents: forking from entries that don't exist or sessions that
  # don't exist.
  Scenario: fork at an unknown entry or session is rejected
    When I call "session::fork" with:
      """
      { "session_id": "s_001", "entry_id": "ghost" }
      """
    Then the call fails with code "session/entry_not_found"
    When I call "session::fork" with:
      """
      { "session_id": "s_404", "entry_id": "e_001" }
      """
    Then the call fails with code "session/not_found"
