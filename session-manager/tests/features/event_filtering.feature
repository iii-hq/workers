@pure
Feature: trigger binding configs — the per-subscriber filter contract

  Contract (session-manager.md § Triggers): the emitting worker
  evaluates each binding's config — session_id equality, roles
  membership (message events only), and metadata subset-equality
  against SessionMeta.metadata (the tenancy hook). Malformed configs
  are rejected AT REGISTRATION so a typo'd filter fails loudly instead
  of silently receiving nothing (or everything).

  These scenarios drive the same SubscriberSet/Emitter code the engine
  routes registrations through in production.

  # Prevents: a per-session chat view receiving every other session's
  # messages.
  Scenario: session_id filters message events to one session
    Given a bare session
    And a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::s1" with config:
      """
      { "session_id": "s_001" }
      """
    When I append a user message "mine" to "s_001"
    And I append a user message "other" to "s_002"
    Then function "ui::s1" received 1 delivery
    And delivery 0 to "ui::s1" has "session_id" = "s_001"

  # Prevents: a roles=["assistant"] live-render binding receiving user
  # messages or custom entries.
  Scenario: roles filters to matching roles and never matches custom entries
    Given a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::assistant" with config:
      """
      { "roles": ["assistant"] }
      """
    When I append a user message "q" to "s_001"
    And I append an empty assistant message to "s_001"
    And I append a custom entry of type "compaction" to "s_001"
    Then function "ui::assistant" received 1 delivery
    And delivery 0 to "ui::assistant" has "message.role" = "assistant"

  # Prevents: confusing role:"custom" MESSAGES (transcript items) with
  # kind:"custom" ENTRIES (bookkeeping) — a roles=["custom"] binding
  # matches the former, never the latter.
  Scenario: roles custom matches custom messages but not custom entries
    Given a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::custom" with config:
      """
      { "roles": ["custom"] }
      """
    When I call "session::append" with:
      """
      { "session_id": "s_001",
        "message": { "role": "custom", "custom_type": "notice", "content": [], "timestamp": 1 } }
      """
    And I append a custom entry of type "compaction" to "s_001"
    Then function "ui::custom" received 1 delivery
    And delivery 0 to "ui::custom" has "message.custom_type" = "notice"

  # Prevents: multi-tenant leaks on message events — the metadata filter
  # must consult the session's metadata, which the payload itself does
  # not carry.
  Scenario: metadata filters message events by the session's metadata
    Given a session created with:
      """
      { "metadata": { "owner": "u_1" } }
      """
    And a session created with:
      """
      { "metadata": { "owner": "u_2" } }
      """
    And a binding "b1" on "session::message_added" delivering to "obs::u1" with config:
      """
      { "metadata": { "owner": "u_1" } }
      """
    When I append a user message "mine" to "s_001"
    And I append a user message "other" to "s_002"
    Then function "obs::u1" received 1 delivery
    And delivery 0 to "obs::u1" has "session_id" = "s_001"

  # Prevents: AND-semantics regressions — all supplied filters must hold
  # simultaneously, not any-of.
  Scenario: combined session_id and roles must both match
    Given a bare session
    And a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::combo" with config:
      """
      { "session_id": "s_001", "roles": ["assistant"] }
      """
    When I append an empty assistant message to "s_002"
    And I append a user message "wrong role" to "s_001"
    Then function "ui::combo" received no deliveries
    When I append an empty assistant message to "s_001"
    Then function "ui::combo" received 1 delivery

  # Prevents: one matching event fanning out to only one of several
  # matching bindings (each binding is independent, even to the same
  # function).
  Scenario: every matching binding receives its own delivery
    Given a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::recv" with config:
      """
      {}
      """
    And a binding "b2" on "session::message_added" delivering to "ui::recv" with config:
      """
      { "session_id": "s_001" }
      """
    When I append a user message "fan out" to "s_001"
    Then function "ui::recv" received 2 deliveries

  # Prevents: unregistered bindings continuing to receive events.
  Scenario: an unbound subscriber stops receiving
    Given a bare session
    And a binding "b1" on "session::message_added" delivering to "ui::recv" with config:
      """
      {}
      """
    Given a user message "before" appended to "s_001"
    When I unbind "b1" from "session::message_added"
    And I append a user message "after" to "s_001"
    Then function "ui::recv" received 1 delivery

  # Prevents: typo'd configs silently registering and then matching
  # nothing — the classic "why is my UI not updating" failure.
  Scenario: a config with an unknown field is rejected at registration
    When I try to bind "bad" on "session::message_added" delivering to "x" with config:
      """
      { "sesion_id": "s_001" }
      """
    Then the binding registration fails mentioning "unknown field"

  # Prevents: roles sneaking onto trigger types that have no role
  # semantics.
  Scenario: roles on status_changed is rejected
    When I try to bind "bad" on "session::status_changed" delivering to "x" with config:
      """
      { "roles": ["assistant"] }
      """
    Then the binding registration fails mentioning "unknown field"

  # Prevents: per-session filters sneaking onto session::created, which
  # the spec defines as unfiltered (metadata aside).
  Scenario: session_id on created is rejected
    When I try to bind "bad" on "session::created" delivering to "x" with config:
      """
      { "session_id": "s_001" }
      """
    Then the binding registration fails mentioning "unknown field"

  # Prevents: invalid role values registering as never-matching filters.
  Scenario: an invalid role value is rejected
    When I try to bind "bad" on "session::message_updated" delivering to "x" with config:
      """
      { "roles": ["robot"] }
      """
    Then the binding registration fails mentioning "invalid session::message_updated config"

  # Prevents: null configs (engine default) being treated differently
  # from {}.
  Scenario: an empty config receives everything
    Given a binding "b1" on "session::created" delivering to "obs::all" with config:
      """
      {}
      """
    When I call "session::create" with "{}"
    And I call "session::create" with:
      """
      { "metadata": { "owner": "u_9" } }
      """
    Then function "obs::all" received 2 deliveries
