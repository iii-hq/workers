@engine
Feature: engine reactivity — real trigger bindings receive filtered events

  The full reactive loop over a LIVE engine: a subscriber registers a
  function and binds it to a session::* trigger type with a config; the
  engine routes the registration into this worker's TriggerHandler; a
  mutation then fans matched events back out through the engine to the
  subscriber function (TriggerAction::Void, at-least-once).

  This is the spec's sequence diagram (session-manager.md § Reactivity
  model) running for real — bind once, render live, no polling.

  # Prevents: the wire-level subscribe -> mutate -> deliver loop
  # breaking anywhere along engine routing, config filtering, or fan-out.
  Scenario: a streamed turn reaches filtered subscribers live
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    And an engine subscriber "R_DELTAS" on "session::message-updated" with config:
      """
      { "roles": ["assistant"], "metadata": { "test_run": "${M1}" } }
      """
    And an engine subscriber "R_STATUS" on "session::status-changed" with config:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And an engine subscriber "R_USER_ONLY" on "session::message-added" with config:
      """
      { "roles": ["user"], "metadata": { "test_run": "${M1}" } }
      """

    # -- the driver flow --
    When over the engine I call "session::create" with:
      """
      { "title": "live turn", "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"
    And over the engine I call "session::set-status" with:
      """
      { "session_id": "${S1}", "status": "working" }
      """
    And over the engine I call "session::append" with:
      """
      { "session_id": "${S1}",
        "message": { "role": "user", "content": [{ "type": "text", "text": "go" }], "timestamp": 1 } }
      """
    And over the engine I call "session::append" with:
      """
      { "session_id": "${S1}", "entry_id": "reply-1",
        "message": { "role": "assistant", "content": [], "stop_reason": "end",
                     "model": "m", "provider": "p", "timestamp": 2 } }
      """
    And over the engine I call "session::update-message" with:
      """
      { "session_id": "${S1}", "entry_id": "reply-1",
        "content": [{ "type": "text", "text": "It" }] }
      """
    And over the engine I call "session::update-message" with:
      """
      { "session_id": "${S1}", "entry_id": "reply-1",
        "content": [{ "type": "text", "text": "It works." }] }
      """
    And over the engine I call "session::set-status" with:
      """
      { "session_id": "${S1}", "status": "done" }
      """

    # -- assistant deltas: both updates, monotonic revisions --
    Then within 10 seconds the engine subscriber "R_DELTAS" receives at least 2 deliveries
    And every engine delivery to "R_DELTAS" has "session_id" = "${S1}"
    And every engine delivery to "R_DELTAS" has "entry_id" = "reply-1"
    And some engine delivery to "R_DELTAS" has "revision" = 1
    And some engine delivery to "R_DELTAS" has "revision" = 2
    And some engine delivery to "R_DELTAS" has "message.content.0.text" = "It works."

    # -- status frames: working then done --
    And within 10 seconds the engine subscriber "R_STATUS" receives at least 2 deliveries
    And some engine delivery to "R_STATUS" has "status" = "working"
    And some engine delivery to "R_STATUS" has "status" = "done"
    And some engine delivery to "R_STATUS" has "previous_status" = "working"

    # -- the roles filter held: the assistant append never reached the
    #    user-only subscriber --
    And within 10 seconds the engine subscriber "R_USER_ONLY" receives at least 1 delivery
    And after 1 second the engine subscriber "R_USER_ONLY" has received exactly 1 delivery
    And every engine delivery to "R_USER_ONLY" has "message.role" = "user"

  # Prevents: session_id-scoped subscribers receiving another session's
  # events when both sessions mutate concurrently.
  Scenario: a session_id-scoped subscriber only sees its session
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    Given over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"
    And over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S2"
    And an engine subscriber "R_S1" on "session::message-added" with config:
      """
      { "session_id": "${S1}" }
      """
    When over the engine I call "session::append" with:
      """
      { "session_id": "${S2}",
        "message": { "role": "user", "content": [{ "type": "text", "text": "noise" }], "timestamp": 1 } }
      """
    And over the engine I call "session::append" with:
      """
      { "session_id": "${S1}",
        "message": { "role": "user", "content": [{ "type": "text", "text": "signal" }], "timestamp": 2 } }
      """
    Then within 10 seconds the engine subscriber "R_S1" receives at least 1 delivery
    And after 1 second the engine subscriber "R_S1" has received exactly 1 delivery
    And every engine delivery to "R_S1" has "session_id" = "${S1}"
    And some engine delivery to "R_S1" has "message.content.0.text" = "signal"

  # Prevents: deletions going dark for tenancy-filtered dashboards (the
  # meta row is already gone when the event must still match).
  Scenario: a deleted session still notifies its metadata-filtered subscriber
    Given the iii engine is reachable
    And a fresh unique marker as "M1"
    Given over the engine I call "session::create" with:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    And over the engine I alias the response field "session_id" as "S1"
    And an engine subscriber "R_DEL" on "session::deleted" with config:
      """
      { "metadata": { "test_run": "${M1}" } }
      """
    When over the engine I call "session::delete" with:
      """
      { "session_id": "${S1}" }
      """
    Then the engine response field "deleted" is true
    And within 10 seconds the engine subscriber "R_DEL" receives at least 1 delivery
    And every engine delivery to "R_DEL" has "session_id" = "${S1}"
