@pure
Feature: observer filters — what each observer of a session event receives

  Contract (session-manager.md § Reactivity model): the six session::on-*
  event functions take no args. iii applies each observer's filters to
  every call: input.session_id, input.message.role (message events) and
  input.session_metadata.<key>. Every event carries session_metadata, the
  session's SessionMeta.metadata (the tenancy hook), so a tenant filter
  needs no lookup. The worker calls an event function only while
  something observes it.

  These scenarios drive the production Emitter; the scenario's recorder
  applies each observer's filters with iii's filter language.

  # Prevents: a per-session chat view receiving every other session's
  # messages.
  Scenario: an input.session_id filter selects one session's message events
    Given a bare session
    And a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::s1" with filters:
      """
      { "input.session_id": { "$eq": "s_001" } }
      """
    When I append a user message "mine" to "s_001"
    And I append a user message "other" to "s_002"
    Then function "ui::s1" received 1 delivery
    And delivery 0 to "ui::s1" has "session_id" = "s_001"

  # Prevents: a live-render observer of assistant messages receiving user
  # messages or custom entries.
  Scenario: a role filter selects matching roles and never matches custom entries
    Given a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::assistant" with filters:
      """
      { "input.message.role": { "$in": ["assistant"] } }
      """
    When I append a user message "q" to "s_001"
    And I append an empty assistant message to "s_001"
    And I append a custom entry of type "compaction" to "s_001"
    Then function "ui::assistant" received 1 delivery
    And delivery 0 to "ui::assistant" has "message.role" = "assistant"

  # Prevents: confusing role:"custom" MESSAGES (transcript items) with
  # kind:"custom" ENTRIES (bookkeeping) — a role filter on "custom"
  # matches the former, never the latter (custom entries carry no message).
  Scenario: a custom role filter matches custom messages but not custom entries
    Given a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::custom" with filters:
      """
      { "input.message.role": { "$in": ["custom"] } }
      """
    When I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "custom", "custom_type": "notice", "content": [], "timestamp": 1 } }
      """
    And I append a custom entry of type "compaction" to "s_001"
    Then function "ui::custom" received 1 delivery
    And delivery 0 to "ui::custom" has "message.custom_type" = "notice"

  # Prevents: multi-tenant leaks on message events — the event must carry
  # the session's metadata for a tenant filter to match on.
  Scenario: a session_metadata filter selects message events by the session's metadata
    Given a session created with:
      """
      { "metadata": { "owner": "u_1" } }
      """
    And a session created with:
      """
      { "metadata": { "owner": "u_2" } }
      """
    And an observer "b1" of "session::on-message-added" delivering to "obs::u1" with filters:
      """
      { "input.session_metadata.owner": { "$eq": "u_1" } }
      """
    When I append a user message "mine" to "s_001"
    And I append a user message "other" to "s_002"
    Then function "obs::u1" received 1 delivery
    And delivery 0 to "obs::u1" has "session_id" = "s_001"
    And delivery 0 to "obs::u1" has "session_metadata.owner" = "u_1"

  # Prevents: a session without metadata delivering a session_metadata
  # field that a filter could mistake for a match.
  Scenario: an event of a session without metadata carries no session_metadata
    Given a bare session
    And an observer "b1" of "session::on-message-added" delivering to "obs::all"
    When I append a user message "plain" to "s_001"
    Then function "obs::all" received 1 delivery
    And delivery 0 to "obs::all" has no field "session_metadata"

  # Prevents: AND-semantics regressions — all supplied filters must hold
  # simultaneously, not any-of.
  Scenario: combined session_id and role filters must both match
    Given a bare session
    And a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::combo" with filters:
      """
      {
        "input.session_id": { "$eq": "s_001" },
        "input.message.role": { "$in": ["assistant"] }
      }
      """
    When I append an empty assistant message to "s_002"
    And I append a user message "wrong role" to "s_001"
    Then function "ui::combo" received no deliveries
    When I append an empty assistant message to "s_001"
    Then function "ui::combo" received 1 delivery

  # Prevents: one matching event reaching only one of several matching
  # observers (each observer is independent, even with the same target).
  Scenario: every matching observer receives its own delivery
    Given a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::recv"
    And an observer "b2" of "session::on-message-added" delivering to "ui::recv" with filters:
      """
      { "input.session_id": { "$eq": "s_001" } }
      """
    When I append a user message "fan out" to "s_001"
    Then function "ui::recv" received 2 deliveries
    And "session::on-message-added" was called 1 time

  # Prevents: disposed observers continuing to receive events.
  Scenario: a disposed observer stops receiving
    Given a bare session
    And an observer "b1" of "session::on-message-added" delivering to "ui::recv"
    Given a user message "before" appended to "s_001"
    When I dispose observer "b1"
    And I append a user message "after" to "s_001"
    Then function "ui::recv" received 1 delivery

  # Prevents: serializing every streaming delta for nobody — the worker
  # calls an event function only while something observes it.
  Scenario: an unobserved event function is never called
    Given a bare session
    And an observer "b1" of "session::on-status-changed" delivering to "ui::status"
    When I append an empty assistant message to "s_001"
    Then "session::on-message-added" was not called
    And "session::on-created" was not called
    And no deliveries were recorded

  # Prevents: an observer without filters missing events.
  Scenario: an observer without filters receives everything
    Given an observer "b1" of "session::on-created" delivering to "obs::all"
    When I call "session::create" with "{}"
    And I call "session::create" with:
      """
      { "metadata": { "owner": "u_9" } }
      """
    Then function "obs::all" received 2 deliveries
