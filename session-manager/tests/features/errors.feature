@pure
Feature: error codes — stable session/* codes on the bus

  Contract (README.md § Error conventions): every error crossing the
  bus carries a stable snake_case, worker-prefixed code. Callers branch
  on these codes programmatically, so each one is pinned here. Reads
  return null for unknown ids (get / get_message) — only mutations and
  path reads reject.

  # Prevents: unknown-session errors mutating into panics, silent
  # successes, or code drift across the mutation surface.
  Scenario: every mutation on an unknown session reports session/not_found
    When I call "session::set-meta" with:
      """
      { "session_id": "ghost", "title": "x" }
      """
    Then the call fails with code "session/not_found"
    When I call "session::set-status" with:
      """
      { "session_id": "ghost", "status": "working" }
      """
    Then the call fails with code "session/not_found"
    When I call "session::append-many" with:
      """
      { "session_id": "ghost", "messages": [ { "role": "user", "content": [], "timestamp": 1 } ] }
      """
    Then the call fails with code "session/not_found"
    When I call "session::update-message" with:
      """
      { "session_id": "ghost", "entry_id": "e", "content": [] }
      """
    Then the call fails with code "session/not_found"
    When I call "session::messages" with:
      """
      { "session_id": "ghost" }
      """
    Then the call fails with code "session/not_found"
    When I call "session::fork" with:
      """
      { "session_id": "ghost", "entry_id": "e" }
      """
    Then the call fails with code "session/not_found"
    When I call "session::set-active-leaf" with:
      """
      { "session_id": "ghost", "entry_id": "e" }
      """
    Then the call fails with code "session/not_found"

  # Prevents: pure reads erroring where the spec promises null.
  Scenario: reads return null for unknown ids
    When I call "session::get" with:
      """
      { "session_id": "ghost" }
      """
    Then the call succeeds
    And the response is null
    Given a bare session
    When I call "session::get-message" with:
      """
      { "session_id": "s_001", "entry_id": "ghost" }
      """
    Then the call succeeds
    And the response is null

  # Prevents: malformed requests reaching the domain logic — serde
  # rejects them at the boundary with a deserialization error.
  Scenario: a structurally invalid request is rejected before any logic runs
    Given a binding "b1" on "session::created" delivering to "obs::all" with config:
      """
      {}
      """
    When I call "session::create" with:
      """
      { "title": 12345 }
      """
    Then the call fails with code "invalid request"
    And function "obs::all" received no deliveries

  # Prevents: unknown enum values (status, role, order) being coerced.
  Scenario: unknown enum values are rejected
    Given a bare session
    When I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "halted" }
      """
    Then the call fails with code "invalid request"
    When I call "session::list" with:
      """
      { "order": "alphabetical" }
      """
    Then the call fails with code "invalid request"
