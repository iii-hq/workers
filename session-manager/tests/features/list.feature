@pure
Feature: session::list — pagination, ordering and filters

  Contract (session-manager.md § session::list): lists sessions with an
  opaque cursor, default order updated_desc (most recently active
  first), optional status filter, and a metadata subset-equality filter
  (the tenancy hook). Ties on the sort key break deterministically by
  session id so pagination never skips or repeats.

  # Prevents: a dashboard's "recent sessions" list ignoring activity —
  # updated_desc must surface the most recently *touched* session first.
  Scenario: default order is updated_desc and reflects activity
    Given a session created with title "first"
    And the clock advances by 10 ms
    And a session created with title "second"
    And the clock advances by 10 ms
    When I append a user message "wake up" to "s_001"
    And I call "session::list" with "{}"
    Then the response field "sessions" has length 2
    And the response field "sessions.0.session_id" is "s_001"
    And the response field "sessions.1.session_id" is "s_002"

  # Prevents: created_asc / created_desc silently both returning the
  # same order.
  Scenario: created_asc and created_desc are mirror orders
    Given a session created with title "a"
    And the clock advances by 10 ms
    And a session created with title "b"
    When I call "session::list" with:
      """
      { "order": "created_asc" }
      """
    Then the response field "sessions.0.session_id" is "s_001"
    When I call "session::list" with:
      """
      { "order": "created_desc" }
      """
    Then the response field "sessions.0.session_id" is "s_002"

  # Prevents: status filters returning foreign statuses (the dashboard
  # "which sessions are working" view).
  Scenario: status filters to exactly that status
    Given a bare session
    And a bare session
    And I call "session::set_status" with:
      """
      { "session_id": "s_002", "status": "working" }
      """
    When I call "session::list" with:
      """
      { "status": "working" }
      """
    Then the response field "sessions" has length 1
    And the response field "sessions.0.session_id" is "s_002"

  # Prevents: tenancy leaks through list — the metadata filter must be
  # subset equality, not best-effort.
  Scenario: metadata filters by subset equality
    Given a session created with:
      """
      { "metadata": { "owner": "u_1", "channel": "web" } }
      """
    And a session created with:
      """
      { "metadata": { "owner": "u_1", "channel": "tg" } }
      """
    And a session created with:
      """
      { "metadata": { "owner": "u_2", "channel": "web" } }
      """
    And a bare session
    When I call "session::list" with:
      """
      { "metadata": { "owner": "u_1" } }
      """
    Then the response field "sessions" has length 2
    When I call "session::list" with:
      """
      { "metadata": { "owner": "u_1", "channel": "tg" } }
      """
    Then the response field "sessions" has length 1
    And the response field "sessions.0.session_id" is "s_002"

  # Prevents: pagination skipping or repeating sessions when walking
  # pages — even with identical timestamps (id tie-break).
  Scenario: cursor pagination is gapless under timestamp ties
    Given a bare session
    And a bare session
    And a bare session
    And a bare session
    And a bare session
    When I call "session::list" with:
      """
      { "limit": 2, "order": "created_asc" }
      """
    Then the response field "sessions" has length 2
    And the response field "sessions.0.session_id" is "s_001"
    And the response field "sessions.1.session_id" is "s_002"
    And I alias the response field "next_cursor" as "C1"
    When I call "session::list" with:
      """
      { "limit": 2, "order": "created_asc", "cursor": "${C1}" }
      """
    Then the response field "sessions.0.session_id" is "s_003"
    And the response field "sessions.1.session_id" is "s_004"
    And I alias the response field "next_cursor" as "C2"
    When I call "session::list" with:
      """
      { "limit": 2, "order": "created_asc", "cursor": "${C2}" }
      """
    Then the response field "sessions" has length 1
    And the response field "sessions.0.session_id" is "s_005"
    And the response has no field "next_cursor"

  # Prevents: cursors silently "working" across a different ordering and
  # returning a garbage page.
  Scenario: a cursor is bound to the order it was issued for
    Given a bare session
    And a bare session
    And a bare session
    Given I call "session::list" with:
      """
      { "limit": 1, "order": "created_asc" }
      """
    And I alias the response field "next_cursor" as "C1"
    When I call "session::list" with:
      """
      { "limit": 1, "order": "created_desc", "cursor": "${C1}" }
      """
    Then the call fails with code "session/invalid_cursor"

  # Prevents: unbounded page sizes blowing up consumers; limits clamp
  # into [1, max_list_limit].
  Scenario: limit zero clamps to one
    Given a bare session
    And a bare session
    When I call "session::list" with:
      """
      { "limit": 0 }
      """
    Then the response field "sessions" has length 1
