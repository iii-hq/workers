@pure
Feature: session::set_active_leaf — branch switching within a session

  Contract (session-manager.md § session::set_active_leaf): moves the
  active path to end at a given entry (switching to a non-leaf makes
  the chain above it the active path). Subsequent session::append
  without parent_id chains from there. Entries on abandoned branches
  are never deleted — they stay readable via get_message and
  from_entry_id views.

  Background:
    Given a bare session
    And a user message "one" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a user message "three" appended to "s_001"

  # Prevents: branch switches silently landing on the wrong entry.
  Scenario: switching to a non-leaf truncates the active path
    When I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then the call succeeds
    And the response field "active_leaf" is "e_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.entry_id" is "e_001"

  # Prevents: appends after a branch switch chaining from the OLD leaf,
  # which would silently merge the branches back together.
  Scenario: append after a switch creates a sibling branch
    Given I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    When I append a user message "two-alt" to "s_001"
    Then the response field "entry_id" is "e_004"
    And the response field "parent_id" is "e_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_004"

  # Prevents: abandoned branches being garbage-collected or hidden from
  # explicit reads — forks and audits depend on them.
  Scenario: the abandoned branch stays fully readable
    Given I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    And a user message "two-alt" appended to "s_001"
    When I call "session::get_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_003" }
      """
    Then the response field "entry.message.content.0.text" is "three"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_003" }
      """
    Then the response field "messages" has length 3
    And the response field "messages.2.entry_id" is "e_003"

  # Prevents: pointing the active path at entries that don't exist.
  Scenario: switching to an unknown entry is rejected
    When I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "ghost" }
      """
    Then the call fails with code "session/entry_not_found"

  # Prevents: set_active_leaf emitting spurious events — the spec gives
  # it none of the six trigger types.
  Scenario: a branch switch emits no events
    Given a binding "b1" on "session::message_added" delivering to "obs::all" with config:
      """
      {}
      """
    And a binding "b2" on "session::meta_updated" delivering to "obs::all" with config:
      """
      {}
      """
    When I call "session::set_active_leaf" with:
      """
      { "session_id": "s_001", "entry_id": "e_001" }
      """
    Then function "obs::all" received no deliveries
