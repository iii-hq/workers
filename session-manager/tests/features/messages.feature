@pure
Feature: session::messages — load the active path, oldest first

  Contract (session-manager.md § session::messages): returns the active
  path as messages paired with entry ids, oldest first (the parent
  chain IS the order). By default only kind:"message" entries are
  returned; include_custom interleaves kind:"custom" entries at their
  path position (how the harness finds its compaction record).
  Supports role filtering, from_entry_id branch views, and cursor
  pagination.

  Background:
    Given a bare session

  # Prevents: a fresh session erroring instead of returning empty.
  Scenario: an empty session returns an empty page
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the call succeeds
    And the response field "messages" has length 0
    And the response has no field "next_cursor"

  # Prevents: transcript order regressions — newest-first or insertion
  # order would scramble every consumer's rendering.
  Scenario: messages come back oldest first with their entry ids
    Given a user message "one" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a user message "three" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 3
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.0.message.content.0.text" is "one"
    And the response field "messages.1.entry_id" is "e_002"
    And the response field "messages.2.entry_id" is "e_003"
    And the response field "messages.2.message.content.0.text" is "three"

  # Prevents: role filters leaking other roles (a context builder asking
  # for user turns must never receive assistant output).
  Scenario: roles filters to matching message roles only
    Given a user message "q1" appended to "s_001"
    And an empty assistant message appended to "s_001"
    And a user message "q2" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "roles": ["user"] }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_003"

  # Prevents: pagination gaps or overlaps — a chat UI walking pages must
  # see every message exactly once.
  Scenario: cursor pagination walks the path without gaps or overlaps
    Given a user message "m1" appended to "s_001"
    And a user message "m2" appended to "s_001"
    And a user message "m3" appended to "s_001"
    And a user message "m4" appended to "s_001"
    And a user message "m5" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "limit": 2 }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_002"
    And I alias the response field "next_cursor" as "C1"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "limit": 2, "cursor": "${C1}" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_003"
    And the response field "messages.1.entry_id" is "e_004"
    And I alias the response field "next_cursor" as "C2"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "limit": 2, "cursor": "${C2}" }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.entry_id" is "e_005"
    And the response has no field "next_cursor"

  # Prevents: accepting forged/stale cursors and returning garbage pages.
  Scenario: a malformed cursor is rejected
    Given a user message "m1" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "cursor": "!!garbage!!" }
      """
    Then the call fails with code "session/invalid_cursor"

  # Prevents: branch views drifting from the spec — from_entry_id must
  # return the parent chain root -> entry, not the whole session.
  Scenario: from_entry_id returns the chain up to that entry
    Given a user message "one" appended to "s_001"
    And a user message "two" appended to "s_001"
    And a user message "three" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "from_entry_id": "e_002" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_002"

  # Prevents: bookkeeping entries leaking into transcripts by default,
  # or include_custom losing their path position.
  Scenario: custom entries appear only with include_custom, in position
    Given a user message "one" appended to "s_001"
    And a custom entry of type "compaction" appended to "s_001"
    And a user message "two" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.0.entry_id" is "e_001"
    And the response field "messages.1.entry_id" is "e_003"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "include_custom": true }
      """
    Then the response field "messages" has length 3
    And the response field "messages.1.entry_id" is "e_002"
    And the response field "messages.1.custom.custom_type" is "compaction"
    And the response has no field "messages.1.message"

  # Prevents: ambiguity when both roles and include_custom are set — a
  # roles filter is an explicit narrowing to message roles.
  Scenario: a roles filter drops custom entries even with include_custom
    Given a user message "one" appended to "s_001"
    And a custom entry of type "compaction" appended to "s_001"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "roles": ["user"], "include_custom": true }
      """
    Then the response field "messages" has length 1
    And the response field "messages.0.entry_id" is "e_001"

  # Prevents: silent empty responses for unknown sessions/entries.
  Scenario: unknown session and unknown from_entry_id are rejected
    When I call "session::messages" with:
      """
      { "session_id": "s_404" }
      """
    Then the call fails with code "session/not_found"
    When I call "session::messages" with:
      """
      { "session_id": "s_001", "from_entry_id": "ghost" }
      """
    Then the call fails with code "session/entry_not_found"
