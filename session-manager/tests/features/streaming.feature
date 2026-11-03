@pure
Feature: the reactivity model — streaming a reply end to end

  Contract (session-manager.md § Reactivity model): streaming uses the
  same primitives as everything else. The driver appends an initially
  empty assistant message (session::on-message-added), then calls
  update_message as tokens arrive (each calling session::on-message-updated with a
  monotonic revision), while set_status frames the turn with working /
  done. A consumer that observes once renders the whole turn live with no
  polling and no separate publish call.

  This is the worker's reason to exist — the full sequence diagram from
  the spec, pinned step by step.

  # Prevents: any regression in the exact event choreography a chat UI
  # depends on: 1 created, 2 status changes, 2 messages added, N
  # message updates with increasing revisions.
  Scenario: a full streamed turn delivers every event a chat UI renders from
    Given an observer "b1" of "session::on-created" delivering to "ui::chat"
    And an observer "b2" of "session::on-message-added" delivering to "ui::chat"
    And an observer "b3" of "session::on-message-updated" delivering to "ui::chat" with filters:
      """
      { "input.message.role": { "$in": ["assistant"] } }
      """
    And an observer "b4" of "session::on-status-changed" delivering to "ui::chat"

    # -- the driver flow --
    When I call "session::create" with:
      """
      { "title": "Weather question" }
      """
    And I append a user message "What's the weather?" to "s_001"
    And I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "working" }
      """
    And I append an empty assistant message to "s_001"
    And I call "session::update-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It" }] }
      """
    And I call "session::update-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It looks" }] }
      """
    And I call "session::update-message" with:
      """
      { "session_id": "s_001", "entry_id": "e_002",
        "content": [{ "type": "text", "text": "It looks sunny." }] }
      """
    And I call "session::set-status" with:
      """
      { "session_id": "s_001", "status": "done" }
      """

    # -- what the consumer saw --
    Then function "ui::chat" received 1 "session::on-created" delivery
    And function "ui::chat" received 2 "session::on-message-added" deliveries
    And function "ui::chat" received 3 "session::on-message-updated" deliveries
    And function "ui::chat" received 2 "session::on-status-changed" deliveries
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
