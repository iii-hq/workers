@pure
Feature: the reactivity model — streaming a reply end to end

  Contract (session-manager.md § Reactivity model): streaming uses the
  same primitives as everything else. The driver appends an initially
  empty assistant message (fires message_added), then calls
  update_message as tokens arrive (each firing message_updated with a
  monotonic revision), while set_status frames the turn with working /
  done. A consumer bound once renders the whole turn live with no
  polling and no separate publish call.

  This is the worker's reason to exist — the full sequence diagram from
  the spec, pinned step by step.

  # Prevents: any regression in the exact event choreography a chat UI
  # depends on: 1 created, 2 status changes, 2 message_added, N
  # message_updated with increasing revisions.
  Scenario: a full streamed turn delivers every event a chat UI renders from
    Given a binding "b1" on "session::created" delivering to "ui::chat" with config:
      """
      {}
      """
    And a binding "b2" on "session::message-added" delivering to "ui::chat" with config:
      """
      {}
      """
    And a binding "b3" on "session::message-updated" delivering to "ui::chat" with config:
      """
      { "roles": ["assistant"] }
      """
    And a binding "b4" on "session::status-changed" delivering to "ui::chat" with config:
      """
      {}
      """

    # -- the driver flow --
    When I call "session::create" with:
      """
      { "title": "Weather question" }
      """
    And I append a user message "What's the weather?" to "s_001"
    And I call "session::set_status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    And I append an empty assistant message to "s_001"
    And I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It" }] }
      """
    And I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It looks" }] }
      """
    And I call "session::update_message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It looks sunny." }] }
      """
    And I call "session::set_status" with:
      """
      { "session_id": "s_001", "status": "done" }
      """

    # -- what the consumer saw --
    Then function "ui::chat" received 1 "session::created" delivery
    And function "ui::chat" received 2 "session::message-added" deliveries
    And function "ui::chat" received 3 "session::message-updated" deliveries
    And function "ui::chat" received 2 "session::status-changed" deliveries
    And delivery 0 to "ui::chat" has "title" = "Weather question"
    And delivery 1 to "ui::chat" has "message.role" = "user"
    And delivery 2 to "ui::chat" has "status" = "working"
    And delivery 3 to "ui::chat" has "message.role" = "assistant"
    And delivery 4 to "ui::chat" has "revision" = 1
    And delivery 5 to "ui::chat" has "revision" = 2
    And delivery 6 to "ui::chat" has "revision" = 3
    And delivery 6 to "ui::chat" has "message.content.0.text" = "It looks sunny."
    And delivery 7 to "ui::chat" has "status" = "done"
    And delivery 7 to "ui::chat" has "previous_status" = "working"

    # -- and what a late joiner reads back --
    When I call "session::messages" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "messages" has length 2
    And the response field "messages.1.message.content.0.text" is "It looks sunny."
    When I call "session::get" with:
      """
      { "session_id": "s_001" }
      """
    Then the response field "meta.status" is "done"
    And the response field "meta.message_count" is 2
