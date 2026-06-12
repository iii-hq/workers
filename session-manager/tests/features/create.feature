@pure
Feature: session::create — create a session at status idle

  Contract (session-manager.md § session::create): a new session starts
  at status "idle" with message_count 0, title/description defaulting to
  "", optional app-defined metadata persisted onto SessionMeta, and
  created_at == updated_at. Every create fires exactly one
  session::created event.

  Deterministic test stack: ids are sequential (s_001, e_001, ...) and
  the clock starts at 1000000 ms, so scenarios assert exact values.

  # Prevents: sessions born in a non-idle state, or with phantom
  # messages, which would break consumers that key spinners off status.
  Scenario: defaults of a bare create
    When I call "session::create" with "{}"
    Then the call succeeds
    And the response field "session_id" is "s_001"
    And the response field "meta.session_id" is "s_001"
    And the response field "meta.status" is "idle"
    And the response field "meta.title" is ""
    And the response field "meta.description" is ""
    And the response field "meta.message_count" is 0
    And the response field "meta.created_at" is 1000000
    And the response field "meta.updated_at" is 1000000
    And the response has no field "meta.metadata"
    And the response has no field "meta.forked_from"
    And the response has no field "meta.status_reason"

  # Prevents: dropping caller-supplied title/description/metadata on the
  # floor — metadata is the tenancy hook every filter relies on.
  Scenario: title, description and metadata are persisted
    When I call "session::create" with:
      """
      {
        "title": "Weather question",
        "description": "User asks about today's forecast.",
        "metadata": { "owner": "u_1", "channel": "web" }
      }
      """
    Then the call succeeds
    And the response field "meta.title" is "Weather question"
    And the response field "meta.description" is "User asks about today's forecast."
    And the response field "meta.metadata.owner" is "u_1"
    And the response field "meta.metadata.channel" is "web"

  # Prevents: id collisions between sessions created back to back.
  Scenario: consecutive creates produce distinct session ids
    When I call "session::create" with "{}"
    And I call "session::create" with "{}"
    Then the response field "session_id" is "s_002"

  # Prevents: silent creates — consumers bind session::created to render
  # new conversations live; a create that does not emit is invisible.
  Scenario: create fires exactly one session::created event with the full payload
    Given a binding "b1" on "session::created" delivering to "obs::created" with config:
      """
      {}
      """
    When I call "session::create" with:
      """
      { "title": "T", "description": "D", "metadata": { "owner": "u_1" } }
      """
    Then function "obs::created" received 1 "session::created" delivery
    And delivery 0 to "obs::created" has "session_id" = "s_001"
    And delivery 0 to "obs::created" has "title" = "T"
    And delivery 0 to "obs::created" has "description" = "D"
    And delivery 0 to "obs::created" has "status" = "idle"
    And delivery 0 to "obs::created" has "created_at" = 1000000
    And delivery 0 to "obs::created" has no field "forked_from"

  # Prevents: leaking other tenants' session creations to a
  # metadata-filtered subscriber.
  Scenario: created events respect the metadata filter
    Given a binding "b1" on "session::created" delivering to "obs::u1" with config:
      """
      { "metadata": { "owner": "u_1" } }
      """
    When I call "session::create" with:
      """
      { "metadata": { "owner": "u_1" } }
      """
    And I call "session::create" with:
      """
      { "metadata": { "owner": "u_2" } }
      """
    And I call "session::create" with "{}"
    Then function "obs::u1" received 1 delivery
    And delivery 0 to "obs::u1" has "session_id" = "s_001"
