@pure
Feature: session::ensure — idempotently ensure a session exists

  Contract (session-manager.md § session::ensure): ensure creates the
  session with the caller-chosen id when missing (firing
  session::created) and is a pure read when it already exists — no
  mutation, no event, `created: false`. title/description/metadata are
  applied ONLY on creation. This is what lets webhook-driven consumers
  (Telegram, Slack) map a platform chat id onto a session without
  racing themselves.

  # Prevents: ensure failing to create, or creating without announcing —
  # a consumer that binds session::created must see ensured sessions too.
  Scenario: ensure creates a missing session and fires created
    Given a binding "b1" on "session::created" delivering to "obs::created" with config:
      """
      {}
      """
    When I call "session::ensure" with:
      """
      { "session_id": "chat-42", "title": "TG chat", "metadata": { "telegram_chat_id": 42 } }
      """
    Then the call succeeds
    And the response field "created" is true
    And the response field "meta.session_id" is "chat-42"
    And the response field "meta.title" is "TG chat"
    And the response field "meta.status" is "idle"
    And function "obs::created" received 1 "session::created" delivery
    And delivery 0 to "obs::created" has "session_id" = "chat-42"

  # Prevents: a redelivered webhook overwriting an existing session's
  # title/metadata or double-firing created (duplicate UI entries).
  Scenario: ensure of an existing session mutates nothing and fires nothing
    Given a binding "b1" on "session::created" delivering to "obs::created" with config:
      """
      {}
      """
    Given I call "session::ensure" with:
      """
      { "session_id": "chat-42", "title": "original", "metadata": { "owner": "u_1" } }
      """
    When I call "session::ensure" with:
      """
      { "session_id": "chat-42", "title": "SHOULD NOT APPLY", "metadata": { "owner": "ATTACKER" } }
      """
    Then the call succeeds
    And the response field "created" is false
    And the response field "meta.title" is "original"
    And the response field "meta.metadata.owner" is "u_1"
    And function "obs::created" received 1 "session::created" delivery

  # Prevents: ensure quietly accepting an unusable empty id.
  Scenario: ensure with an empty session_id is rejected
    When I call "session::ensure" with:
      """
      { "session_id": "" }
      """
    Then the call fails with code "session/invalid_request"

  # Prevents: ensure-created sessions drifting from create-created ones
  # (both must start idle with zero messages).
  Scenario: an ensured session has the same fresh shape as a created one
    When I call "session::ensure" with:
      """
      { "session_id": "chat-42" }
      """
    Then the response field "meta.message_count" is 0
    And the response field "meta.status" is "idle"
    And the response field "meta.created_at" is 1000000
    And the response field "meta.updated_at" is 1000000
